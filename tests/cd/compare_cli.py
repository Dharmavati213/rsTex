#!/usr/bin/env -S python3 -I
r"""The commute command against real PGF: every diagram of tests/cd and a sample of the public manifest, each as `standalone` draws it.

    python3 -I tests/cd/compare_cli.py --commute target/release/commute [--sample 300] [--seed 1] [--mode library|preamble|both] [--dpi 400] [-j 4] [--filter REGEX] [--show 30]
    python3 -I tests/cd/compare_cli.py --commute C --files         # a file of many diagrams gives what each alone gives, byte for byte
    python3 -I tests/cd/compare_cli.py --commute C --check-negative   # that this tool does fail when a diagram is off by a little
    python3 -I tests/cd/compare_cli.py --commute C --tests-cd [--mode library] [--files]   # the diagrams in tests/cd as they stand, with no manifest: what a system with none of target/ has

Where the diagrams come from: tests/corpus/manifests/cd.json, whose documents tests/corpus/cd/extract.py wrote, one diagram per page with its document's own preamble.  All of tests/cd is there as
`cd/tests-cd-units/...`, and of the rest a sample by seed.  Nothing of the owner's is read: this tool does not open the .local manifest.

The two ways of the command are judged apart, since they are two paths.

  library   the diagram alone, as `commute FILE` has it, against `\documentclass[border=0pt]{standalone}` with amsmath, amssymb and tikz-cd, and the library that squiggly arrows ask for.  A diagram that needs its
            document's preamble fails on one side or both: it is counted, and it says nothing.  What only the command declines is counted as that, with the reason.
  preamble  a document of its preamble, class and all, and the diagram: `commute FILE` takes the preamble from there.  The oracle compiles the same with what `standalone` does to the page done by hand, `\pdfpagewidth`,
            `\hoffset` and a `\shipout` of the box: `standalone` itself does not get along with a document's `geometry` or its page headers.  The library mode shows the two are the same.  What fails on the oracle is counted.

Per diagram, against what the oracle draws:

  box       width, height and depth, as `\the` prints them, from `\setbox` in the reference and from `--box`: equal to the scaled point.
  page      `/MediaBox` of the two PDFs, the numbers as they stand: the page is the box, and nothing else, on both.
  pixels    pdftoppm at --dpi on both.  Equal, by default: --max-bad channel values may differ by more than --max-level.  At 400 dpi 0.05 bp is 0.28 pixel.
  svg       rsvg-convert at --dpi against the oracle's raster.  Two renderers do not agree on antialiasing, so each dark pixel on either side must have a dark pixel within --svg-reach pixels on the other: at 400 dpi
            one pixel is 0.18 bp.  Also: there is no <text>, it parses as XML, and its viewBox is the box.  Without rsvg-convert the tool says so, and judges only the rest.
  qpdf      qpdf --check says nothing is wrong, if there is qpdf.

--files runs all diagrams of tests/cd that the library takes in one file, and compares each PDF and SVG with what a run of that diagram alone writes, but for a font's number in its name, which depends on
what was loaded before it: that proves `session_new_pdf`.

Where there is no pdftoppm the tool says so, and judges the rest, and the sums of what the command wrote can show the same as another system's: --sums FILE.

The oracle's PDFs stay under target/cd-cli/ref, by the hash of their source.  Exit status 0 if nothing failed.
"""
import argparse
import collections
import concurrent.futures as cf
import hashlib
import json
import os
import random
import re
import shutil
import subprocess
import sys
import threading
import xml.parsers.expat
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
TARGET = os.path.join(REPO, 'target')
ROOT = os.path.join(TARGET, 'cd-cli')
REF = os.path.join(ROOT, 'ref')
MANIFEST = os.path.join(REPO, 'tests', 'corpus', 'manifests', 'cd.json')
DIAGRAM = re.compile(rb'\\typeout\{\[\[unit-start ([^\]]+)\]\]\}\n\\setbox\\cdunit=\\hbox\{\\begin\{tikzcd\}(.*?)\\end\{tikzcd\}\}\n\\typeout\{\[\[unit ', re.S)
# What says nothing about a preamble: without anything else in it, the diagram is one for the library.
PLAIN_LINE = re.compile(rb'^\s*(%.*)?$|^\s*\\documentclass\{article\}\s*$|^\s*\\pdf(obj)?compresslevel=0[ \\a-z=0]*$|^\s*\\usepackage\{(amsmath|amssymb|amsfonts|tikz-cd|graphicx|xcolor|,)*\}\s*$|^\s*\\usetikzlibrary\{(cd|decorations.pathmorphing|,)*\}\s*$|^\s*\\pagestyle\{[a-z]*\}\s*$')
REF_ENV = {'SOURCE_DATE_EPOCH': '1700000000', 'FORCE_SOURCE_DATE': '1'}


def items(args):
    """(id, preamble with its class, whether that preamble says anything, body), in a fixed order."""
    with open(MANIFEST, encoding='utf-8') as f:
        entries = json.load(f)['entries']
    units, rest = [], []
    for e in entries:
        path = os.path.join(REPO, e['main'])
        if not e.get('units') or not os.path.isfile(path):
            continue
        with open(path, 'rb') as f:
            data = f.read()
        at = data.find(b'\\begin{document}')
        if at < 0:
            continue
        pre = re.sub(rb'\\pdfcompresslevel=0|\\pdfobjcompresslevel=0', b'', data[:at])
        plain = all(PLAIN_LINE.match(line) for line in data[:at].split(b'\n'))
        for m in DIAGRAM.finditer(data):
            it = (e['id'] + '#' + m.group(1).decode(), pre, plain, m.group(2), e)
            (units if e['id'].startswith('cd/tests-cd-units/') else rest).append(it)
    random.Random(args.seed).shuffle(rest)
    chosen = units + rest[:args.sample]
    if args.filter:
        chosen = [x for x in chosen if re.search(args.filter, x[0])]
    return chosen, len(units), len(rest)


def items_of_tests_cd(args):
    """As items(), from tests/cd/*.tex themselves."""
    chosen = []
    for name in sorted(os.listdir(HERE)):
        if not name.endswith('.tex'):
            continue
        with open(os.path.join(HERE, name), 'rb') as f:
            data = f.read()
        at = data.find(b'\\begin{document}')
        if at < 0:
            continue
        pre = re.sub(rb'\\pdfcompresslevel=0|\\pdfobjcompresslevel=0', b'', data[:at])
        plain = all(PLAIN_LINE.match(line) for line in data[:at].split(b'\n'))
        for k, m in enumerate(re.finditer(rb'\\begin\{tikzcd\}(.*?)\\end\{tikzcd\}', data[at:], re.S)):
            chosen.append((f'cd/tests-cd-units/{name[:-4]}#{k}', pre, plain, m.group(1), {'texinputs': ['tests/cd']}))
    if args.filter:
        chosen = [x for x in chosen if re.search(args.filter, x[0])]
    return chosen, len(chosen), 0


NEEDS = b'\\usepackage{amsmath,amssymb}\n\\usepackage{tikz-cd}\n\\usetikzlibrary{decorations.pathmorphing}\n'
BOX = b'\\newbox\\cdunit\\setbox\\cdunit=\\hbox{\\begin{tikzcd}%s\\end{tikzcd}}\\typeout{[[box \\the\\wd\\cdunit\\space\\the\\ht\\cdunit\\space\\the\\dp\\cdunit]]}'


def oracle_tree():
    """Which TeX the oracle's PDFs come from: a tree's PGF draws what it draws, and this directory stays from one run to the next."""
    if not ORACLE_TREE:
        pdflatex = shutil.which('pdflatex') or ''
        p = subprocess.run(['kpsewhich', 'pgf.revision.tex'], capture_output=True, text=True)
        try:
            with open(p.stdout.strip(), 'rb') as f:
                ORACLE_TREE.append(pdflatex.encode() + f.read())
        except OSError:
            ORACLE_TREE.append(pdflatex.encode())
    return ORACLE_TREE[0]


ORACLE_TREE = []
REF_LOCK = threading.Lock()
REF_LOCKS = {}


def reference(item, with_preamble, env):
    """The oracle's run.  (directory, status, box text), kept under target/."""
    ident, pre, _, body, e = item
    if with_preamble:
        src = pre + b'\n' + NEEDS + b'\\begin{document}\n' + BOX % body
        src += (b'\n\\pdfpagewidth=\\wd\\cdunit \\pdfpageheight=\\dimexpr\\ht\\cdunit+\\dp\\cdunit\\relax \\pdfhorigin=1 true in \\pdfvorigin=1 true in \\hoffset=-72.27pt \\voffset=-72.27pt\n'
                b'\\shipout\\box\\cdunit\n\\end{document}\n')
    else:
        src = b'\\documentclass[border=0pt]{standalone}\n' + NEEDS + b'\\begin{document}\n' + BOX % body + b'\\box\\cdunit\n\\end{document}\n'
    key = hashlib.sha1(src + json.dumps(e.get('texinputs', [])).encode() + oracle_tree()).hexdigest()
    d = os.path.join(REF, key)
    done = os.path.join(d, 'done')
    # Two diagrams can be the same text, and two threads must not run in one directory, or read `done` while the other writes it.
    with REF_LOCK:
        lock = REF_LOCKS.setdefault(key, threading.Lock())
    with lock:
        if not os.path.exists(done):
            os.makedirs(d, exist_ok=True)
            with open(os.path.join(d, 'a.tex'), 'wb') as f:
                f.write(src)
            p = subprocess.run(['pdflatex', '-interaction=batchmode', '-no-shell-escape', 'a.tex'], cwd=d, env=tex_env(e, env), capture_output=True, timeout=300)
            with open(done, 'w') as f:
                f.write(str(p.returncode))
    with open(done) as f:
        rc = int(f.read())
    box = None
    log = os.path.join(d, 'a.log')
    if os.path.exists(log):
        with open(log, 'rb') as f:
            text = re.sub(rb'\n(?=[^\n]{0,78}\]\])', b'', f.read())
        m = re.search(rb'\[\[box ([^\]]*)\]\]', text)
        if m:
            box = m.group(1).decode()
    ok = rc == 0 and box is not None and os.path.exists(os.path.join(d, 'a.pdf'))
    return d, ok, box


def tex_env(e, env):
    out = dict(env)
    out.update(REF_ENV)
    dirs = [os.path.join(REPO, p) for p in e.get('texinputs', [])]
    if dirs:
        out['TEXINPUTS'] = os.pathsep.join(dirs) + os.pathsep
    return out


def page_rasters(pdf, dpi):
    p = subprocess.run(['pdftoppm', '-r', str(dpi), pdf], capture_output=True)
    b = p.stdout
    fields, i = [], 2
    while len(fields) < 3:
        while b[i:i + 1].isspace():
            i += 1
        j = i
        while not b[j:j + 1].isspace():
            j += 1
        fields.append(int(b[i:j]))
        i = j
    i += 1
    return fields[0], fields[1], b[i:i + fields[0] * fields[1] * 3]


def raster_compare(x, y, opts):
    if x[:2] != y[:2]:
        return f'pixels: the page is {y[:2]}, real PGF {x[:2]}'
    if x[2] == y[2]:
        return None
    a, b, width = x[2], y[2], x[0]
    stride = width * 3
    bad = 0
    worst = 0
    first = None
    for i in range(0, len(a), stride):
        ra, rb = a[i:i + stride], b[i:i + stride]
        if ra != rb:
            for j, (p, q) in enumerate(zip(ra, rb)):
                if abs(p - q) > opts.max_level:
                    bad += 1
                    worst = max(worst, abs(p - q))
                    first = first or (j // 3, i // stride)
    if bad <= opts.max_bad:
        return None
    return f'pixels: {bad} channel values differ by more than {opts.max_level}, by {worst} at most, first at {first}'


def png_gray(path):
    """A PNG from rsvg-convert over white: width, height, rows of 0 to 255 by luminance."""
    with open(path, 'rb') as f:
        b = f.read()
    i, idat, w, h, ct = 8, [], 0, 0, 0
    while i < len(b):
        n = int.from_bytes(b[i:i + 4], 'big')
        t = b[i + 4:i + 8]
        d = b[i + 8:i + 8 + n]
        if t == b'IHDR':
            w, h = int.from_bytes(d[:4], 'big'), int.from_bytes(d[4:8], 'big')
            ct = d[9]
        elif t == b'IDAT':
            idat.append(d)
        i += 12 + n
    raw = zlib.decompress(b''.join(idat))
    ch = {0: 1, 2: 3, 4: 2, 6: 4}[ct]
    stride = w * ch
    rows, prev, at = [], bytearray(stride), 0
    for _ in range(h):
        f = raw[at]
        line = bytearray(raw[at + 1:at + 1 + stride])
        at += 1 + stride
        if f == 1:
            for k in range(ch, stride):
                line[k] = (line[k] + line[k - ch]) & 255
        elif f == 2:
            for k in range(stride):
                line[k] = (line[k] + prev[k]) & 255
        elif f == 3:
            for k in range(stride):
                line[k] = (line[k] + ((line[k - ch] if k >= ch else 0) + prev[k]) // 2) & 255
        elif f == 4:
            for k in range(stride):
                a, u, c = line[k - ch] if k >= ch else 0, prev[k], prev[k - ch] if k >= ch else 0
                p = a + u - c
                pa, pb, pc = abs(p - a), abs(p - u), abs(p - c)
                line[k] = (line[k] + (a if pa <= pb and pa <= pc else u if pb <= pc else c)) & 255
        prev = line
        rows.append(line)
    out = []
    for line in rows:
        if ch == 4:
            g = bytes((255 * (255 - line[k + 3]) + line[k + 3] * (line[k] * 30 + line[k + 1] * 59 + line[k + 2] * 11) // 100) // 255 for k in range(0, stride, 4))
        elif ch == 3:
            g = bytes((line[k] * 30 + line[k + 1] * 59 + line[k + 2] * 11) // 100 for k in range(0, stride, 3))
        elif ch == 2:
            g = bytes(line[k] for k in range(0, stride, 2))
        else:
            g = bytes(line)
        out.append(g)
    return w, h, out


def ppm_gray(x):
    w, h, b = x
    rows = []
    for r in range(h):
        s = b[r * w * 3:(r + 1) * w * 3]
        rows.append(bytes((s[k] * 30 + s[k + 1] * 59 + s[k + 2] * 11) // 100 for k in range(0, w * 3, 3)))
    return w, h, rows


def dark_set(rows, level):
    return [{i for i, v in enumerate(r) if v < level} for r in rows]


def reach_compare(a, b, reach, level_hard=96, level_soft=224):
    """How many clearly dark pixels of `a` have nothing even a little dark within `reach` on `b`."""
    w, h, ra = a
    _, _, rb = b
    hard, soft = dark_set(ra, level_hard), dark_set(rb, level_soft)
    bad = 0
    first = None
    for y in range(h):
        for x in hard[y]:
            near = False
            for yy in range(max(0, y - reach), min(h, y + reach + 1)):
                s = soft[yy] if yy < len(soft) else ()
                if any((xx in s) for xx in range(x - reach, x + reach + 1)):
                    near = True
                    break
            if not near:
                bad += 1
                first = first or (x, y)
    return bad, first


def svg_compare(svg_path, real_raster, work, opts):
    """Why the SVG is not what the oracle draws, or None."""
    with open(svg_path, 'rb') as f:
        data = f.read()
    if b'<text' in data:
        return 'svg: it has <text>'
    if b'href' in data or b'url(' in data:
        return 'svg: it refers to something outside'
    try:
        xml.parsers.expat.ParserCreate().Parse(data, True)
    except xml.parsers.expat.ExpatError as e:
        return f'svg: not XML: {e}'
    if not opts.rsvg or real_raster is None:
        return None
    png = os.path.join(work, 'svg.png')
    p = subprocess.run([opts.rsvg, '-d', str(opts.dpi), '-p', str(opts.dpi), svg_path, '-o', png], capture_output=True)
    if p.returncode:
        return f'svg: rsvg-convert says {p.stderr[:100]!r}'
    s = png_gray(png)
    r = ppm_gray(real_raster)
    if abs(s[0] - r[0]) > 1 or abs(s[1] - r[1]) > 1:
        return f'svg: rendered {s[:2]}, real PGF {r[:2]}'
    n = min(s[1], r[1])
    s = (s[0], n, s[2][:n])
    r = (r[0], n, r[2][:n])
    for (a, b, side) in ((r, s, 'real PGF has'), (s, r, 'the SVG has')):
        bad, first = reach_compare(a, b, opts.svg_reach)
        if bad > opts.svg_bad:
            return f'svg: {side} {bad} dark pixels with nothing within {opts.svg_reach} on the other, first at {first}'
    return None


def media_box(pdf):
    p = subprocess.run(['qpdf', '--qdf', '--object-streams=disable', pdf, '-'], capture_output=True)
    m = re.search(rb'/MediaBox\s*\[\s*([-0-9. ]+?)\s*\]', p.stdout.replace(b'\n', b' '))
    return m.group(1).split() if m else None


def qpdf_check(pdf):
    p = subprocess.run(['qpdf', '--check', pdf], capture_output=True)
    return None if p.returncode == 0 else 'qpdf: ' + p.stdout.decode('utf-8', 'replace').strip().split('\n')[-1][:80]


def pdf_normal(path, opts):
    """A PDF as qpdf writes it out, but for the digits of a font's name: those depend on what a run loaded before it.  As it stands if there is no qpdf."""
    if not opts.qpdf:
        with open(path, 'rb') as f:
            return f.read()
    p = subprocess.run(['qpdf', '--qdf', '--object-streams=disable', '--deterministic-id', path, '-'], capture_output=True)
    text = re.sub(rb'/F\d+', b'/F', p.stdout)
    # The cross-reference table says where things are, and a name's digits move those.  The rest of the trailer is qpdf's hash of it all.
    at = text.rfind(b'\nxref\n')
    return text[:at] if at >= 0 else text


def run_commute(opts, argv, cwd, env):
    return subprocess.run([opts.commute, '--box'] + argv, cwd=cwd, env=env, capture_output=True, timeout=300)


def one(item, opts, with_preamble):
    ident, pre, plain, body, e = item
    res = {'id': ident, 'fate': None, 'why': None}
    env = {k: v for k, v in os.environ.items() if not k.startswith('RSTEX_')}
    work = os.path.join(ROOT, 'work', hashlib.sha1(((ident) + str(with_preamble)).encode()).hexdigest())
    shutil.rmtree(work, ignore_errors=True)
    os.makedirs(work)
    try:
        d, ok, box = reference(item, with_preamble, env)
        with open(os.path.join(work, 'd.tex'), 'wb') as f:
            if with_preamble:
                f.write(pre + b'\n\\begin{document}\n')
            f.write(b'\\begin{tikzcd}' + body + b'\\end{tikzcd}\n')
            if with_preamble:
                f.write(b'\\end{document}\n')
        p = run_commute(opts, ['d.tex'], work, tex_env(e, env))
        err = p.stderr.decode('utf-8', 'replace').strip()
        pdf, svg = os.path.join(work, 'd.pdf'), os.path.join(work, 'd.svg')
        if not ok:
            res['fate'] = 'oracle fails'
            res['commute_too'] = p.returncode != 0
            return res
        if p.returncode != 0 and not os.path.exists(pdf):
            m = re.search(r'not handled: ([^:]*)', err)
            res['fate'] = 'declined'
            res['why'] = (m.group(1) if m else err[:100])
            return res
        fails = []
        got = p.stdout.decode('utf-8', 'replace').split()
        got = ' '.join(got[-3:]) if len(got) >= 3 else None
        if got != box:
            fails.append(f'box: {got}, real PGF {box}')
        real = os.path.join(d, 'a.pdf')
        if opts.qpdf:
            ma, mb = media_box(real), media_box(pdf)
            if ma != mb:
                fails.append(f'page: {mb}, real PGF {ma}')
            q = qpdf_check(pdf)
            if q:
                fails.append(q)
        ra = None
        if opts.pdftoppm:
            ra, rb = page_rasters(real, opts.dpi), page_rasters(pdf, opts.dpi)
            why = raster_compare(ra, rb, opts)
            if why:
                fails.append(why)
        if os.path.exists(svg):
            why = svg_compare(svg, ra, work, opts)
            if why:
                fails.append(why)
        elif 'no SVG' in err:
            res['no_svg'] = err.partition('no SVG: ')[2].split('\n')[0][:80]
        else:
            fails.append('no SVG, and it does not say why')
        if p.returncode != 0 and 'no SVG' not in err:
            fails.append(f'status {p.returncode}: {err[:100]}')
        res['fate'] = 'fail' if fails else 'pass'
        res['why'] = '; '.join(fails) if fails else None
        if not with_preamble:
            with open(pdf, 'rb') as f:
                res['sums'] = hashlib.sha256(f.read()).hexdigest()
            if os.path.exists(svg):
                with open(svg, 'rb') as f:
                    res['sums'] += ' ' + hashlib.sha256(f.read()).hexdigest()
        return res
    finally:
        shutil.rmtree(work, ignore_errors=True)


def summary(name, results, opts):
    c = collections.Counter(r['fate'] for r in results)
    judged = c['pass'] + c['fail']
    print(f'{name}: {len(results)} diagrams: {c["pass"]} of {judged} that both draw are equal; {c["fail"]} differ, {c["declined"]} declined by the command, {c["oracle fails"]} fail on the oracle')
    no_svg = collections.Counter(r['no_svg'] for r in results if 'no_svg' in r)
    if no_svg:
        print(f'  {sum(no_svg.values())} of those without SVG: ' + '; '.join(f'{v}, {k}' for k, v in no_svg.most_common(4)))
    why = collections.Counter(r['why'] for r in results if r['fate'] == 'declined')
    for k, v in why.most_common(12):
        print(f'  declined {v:4d}  {k}')
    allowed = [r for r in results if r['fate'] == 'fail' and (name, r['id']) in opts.allow]
    for r in [r for r in results if r['fate'] == 'fail' and (name, r['id']) not in opts.allow][:opts.show]:
        print(f'  FAIL {r["id"]}: {r["why"][:220]}')
    if allowed:
        print(f'  {len(allowed)} of those that differ are on {opts.allow_name}, which says why; first {allowed[0]["id"]}: {allowed[0]["why"][:120]}')
    if opts.allow_write:
        with open(opts.allow_write, 'a', newline='\n') as f:
            for r in results:
                if r['fate'] == 'fail':
                    f.write(f'{name} {r["id"]}\n')
    return c['fail'] - len(allowed)


def files_mode(chosen, opts):
    """All that the library takes, in one file, against each alone."""
    env = {k: v for k, v in os.environ.items() if not k.startswith('RSTEX_')}
    work = os.path.join(ROOT, 'files')
    shutil.rmtree(work, ignore_errors=True)
    os.makedirs(work)
    taken = []
    for it in chosen:
        name = f'{len(taken):04d}'
        with open(os.path.join(work, name + '.tex'), 'wb') as f:
            f.write(b'\\begin{tikzcd}' + it[3] + b'\\end{tikzcd}\n')
        p = subprocess.run([opts.commute, name + '.tex'], cwd=work, env=env, capture_output=True)
        if p.returncode == 0:
            taken.append((name, it))
    with open(os.path.join(work, 'all.tex'), 'wb') as f:
        for _, it in taken:
            f.write(b'\\begin{tikzcd}' + it[3] + b'\\end{tikzcd}\n')
    p = subprocess.run([opts.commute, 'all.tex'], cwd=work, env=env, capture_output=True)
    bad = 0
    for k, (name, it) in enumerate(taken):
        for ext in ('pdf', 'svg'):
            many_path = os.path.join(work, f'all-{k + 1}.{ext}') if len(taken) > 1 else os.path.join(work, f'all.{ext}')
            alone_path = os.path.join(work, f'{name}.{ext}')
            if ext == 'pdf':
                alone, many = pdf_normal(alone_path, opts), pdf_normal(many_path, opts) if os.path.exists(many_path) else b''
            else:
                with open(alone_path, 'rb') as f:
                    alone = f.read()
                many = open(many_path, 'rb').read() if os.path.exists(many_path) else b''
            if alone != many:
                bad += 1
                if bad <= opts.show:
                    print(f'  FAIL {it[0]}: {ext} from the file of many is not what it is alone')
    print(f'files: {len(taken)} of {len(chosen)} diagrams in one file, status {p.returncode}: {2 * len(taken) - bad} of {2 * len(taken)} files equal to what the diagram alone writes')
    shutil.rmtree(work, ignore_errors=True)
    return bad + (p.returncode != 0)


def check_negative(opts):
    """A diagram off by 0.05 bp fails on pixels, and an SVG off by 0.5 bp, not quite three pixels, fails on the renderer's: these are what the comparison can see."""
    env = {k: v for k, v in os.environ.items() if not k.startswith('RSTEX_')}
    work = os.path.join(ROOT, 'negative')
    shutil.rmtree(work, ignore_errors=True)
    os.makedirs(work)
    body = b'A \\arrow[r,"f"] \\arrow[d] & B \\arrow[d] \\\\ C \\arrow[r] & D'
    item = ('negative', b'', True, body, {})
    d, ok, _ = reference(item, False, env)
    real = page_rasters(os.path.join(d, 'a.pdf'), opts.dpi)
    bad = 0

    def draw(name, text):
        with open(os.path.join(work, name + '.tex'), 'wb') as f:
            f.write(b'\\begin{tikzcd}' + text + b'\\end{tikzcd}\n')
        subprocess.run([opts.commute, name + '.tex'], cwd=work, env=env, capture_output=True, check=True)
        return page_rasters(os.path.join(work, name + '.pdf'), opts.dpi), os.path.join(work, name + '.svg')

    same, svg = draw('same', body)
    if raster_compare(real, same, opts) or svg_compare(svg, real, work, opts):
        print('the diagram itself differs from real PGF')
        bad += 1
    off, _ = draw('off', body.replace(b'\\arrow[d] & B', b'\\arrow[d,xshift=0.05bp] & B'))
    print('off by 0.05 bp:', raster_compare(real, off, opts))
    if not raster_compare(real, off, opts):
        bad += 1
    if opts.rsvg:
        with open(svg, 'rb') as f:
            text = f.read()
        shifted = os.path.join(work, 'shifted.svg')
        at = text.index(b'>') + 1
        with open(shifted, 'wb') as f:
            f.write(text[:at] + b'<g transform="translate(0.5 0)">' + text[at:].replace(b'</svg>', b'</g></svg>'))
        print('SVG off by 0.5 bp:', svg_compare(shifted, real, work, opts))
        if not svg_compare(shifted, real, work, opts):
            bad += 1
    shutil.rmtree(work, ignore_errors=True)
    print('PASS' if not bad else 'FAIL')
    return 1 if bad else 0


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--commute', required=True)
    ap.add_argument('--sample', type=int, default=300)
    ap.add_argument('--seed', type=int, default=1)
    ap.add_argument('--mode', default='both', choices=['library', 'preamble', 'both'])
    ap.add_argument('--files', action='store_true')
    ap.add_argument('--check-negative', action='store_true')
    ap.add_argument('--dpi', type=int, default=400)
    ap.add_argument('--max-level', type=int, default=0)
    ap.add_argument('--max-bad', type=int, default=0)
    ap.add_argument('--svg-reach', type=int, default=1)
    ap.add_argument('--svg-bad', type=int, default=0)
    ap.add_argument('-j', type=int, default=4)
    ap.add_argument('--filter')
    ap.add_argument('--show', type=int, default=30)
    ap.add_argument('--tests-cd', action='store_true', help='tests/cd/*.tex as they stand, with no manifest')
    ap.add_argument('--sums', metavar='FILE', help='what each diagram of the library mode wrote, to hold against another system')
    ap.add_argument('--allow', metavar='FILE', help='diagrams that may differ, by mode and id: lines of a list that says why')
    ap.add_argument('--allow-write', metavar='FILE', help='appends what differed, in that form')
    ap.add_argument('--no-rsvg', action='store_true', help='SVGs as XML and for what they hold alone, which takes a fraction of the time')
    opts = ap.parse_args()
    opts.commute = os.path.abspath(opts.commute)
    opts.rsvg = None if opts.no_rsvg else shutil.which('rsvg-convert')
    opts.qpdf = shutil.which('qpdf')
    opts.pdftoppm = shutil.which('pdftoppm')
    opts.allow, opts.allow_name = set(), opts.allow
    if opts.allow_name:
        with open(opts.allow_name, encoding='utf-8') as f:
            opts.allow = {tuple(line.split()[:2]) for line in f if line.strip() and not line.startswith('#')}
    if not opts.pdftoppm:
        print('there is no pdftoppm here: the pixels are not compared')
    if not opts.rsvg:
        print('there is no rsvg-convert here: SVGs are checked as XML and for what they hold, and not by what they look like')
    os.makedirs(REF, exist_ok=True)
    if opts.check_negative:
        return check_negative(opts)
    chosen, nunits, nrest = items_of_tests_cd(opts) if opts.tests_cd else items(opts)
    print(f'{len(chosen)} diagrams: {nunits} of tests/cd, and {len(chosen) - nunits if not opts.filter else "some"} of {nrest} others of the public manifest, seed {opts.seed}')
    if opts.files:
        return 1 if files_mode([x for x in chosen if x[0].startswith('cd/tests-cd-units/')], opts) else 0
    failed = 0
    if opts.mode in ('library', 'both'):
        # The library has no preamble: a diagram whose document has one is for the other mode, unless asked by a filter.
        with cf.ThreadPoolExecutor(opts.j) as ex:
            results = list(ex.map(lambda it: one(it, opts, False), chosen))
        failed += summary('library', results, opts)
        if opts.sums:
            os.makedirs(os.path.dirname(os.path.abspath(opts.sums)), exist_ok=True)
            with open(opts.sums, 'w', newline='\n') as f:
                for r in results:
                    if 'sums' in r:
                        f.write(f"{r['id']} {r['sums']}\n")
    if opts.mode in ('preamble', 'both'):
        todo = [x for x in chosen if not x[2]]
        with cf.ThreadPoolExecutor(opts.j) as ex:
            results = list(ex.map(lambda it: one(it, opts, True), todo))
        failed += summary('preamble', results, opts)
    print('PASS' if not failed else 'FAIL')
    return 1 if failed else 0


if __name__ == '__main__':
    sys.exit(main())
