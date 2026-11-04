#!/usr/bin/env -S python3 -I
"""census.py: what the corpus is, from the oracle's side only.  Nothing here runs or reads rstex's results.

    census.py measure [-j 4] [--ids REGEX] [--limit N]    instructions of the oracle on every document, one last pass and one preamble alone: target/bs-design/oracle-counts.jsonl
    census.py build                                       joins that with the manifests, the oracle cache and the oracle's columns of the corpus records: target/bs-design/census.jsonl, tree.json
    census.py diagrams [-j 4]                             every public diagram alone under pdflatex with real tikz-cd: target/bs-design/diagrams.jsonl
    census.py show                                        the distribution, as tests/bench/README.md quotes it: counts and public ids only

`measure` compiles with the system pdflatex or pdftex as tests/corpus/run.py does, its argv, its environment and the oracle's last-pass side files, under user-mode hardware counters from outside
the process, as tools/prof/proflib.py has them.  The preamble alone is the same run with a prelude that ends the job at the hook `begindocument/end`, so no document's source is touched or read.
Counts do not depend on the load of the machine, and a time measured here means nothing: `cpu_ms` is kept for a sanity check and for nothing else.

`build` reads only ORACLE_COLUMNS of a corpus record: anything rstex produced is dropped at the door, and a test in test_select.py holds that.  Of a private document it reads what the oracle's cache
says of it: page and pass counts, the names of the files of the TeX tree it opened, how many files of its own and of which extension.  Its own file names, its text and its log are not read or kept.
Of a public document it also counts a few constructs in its own sources, which is what `src_*` are; a private document has none of those fields.
"""
import os
import sys

# select.py in this directory is not the standard library's select: subprocess needs that one, so this directory must not be on the path.
sys.path[:] = [p for p in sys.path if os.path.abspath(p or os.curdir) != os.path.dirname(os.path.abspath(__file__))]

import argparse  # noqa: E402
import collections  # noqa: E402
import concurrent.futures as cf  # noqa: E402
import hashlib  # noqa: E402
import importlib.util  # noqa: E402
import json  # noqa: E402
import re  # noqa: E402
import secrets  # noqa: E402
import shutil  # noqa: E402
import subprocess  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, os.path.join(REPO, 'tools', 'prof'))
import proflib  # noqa: E402

_spec = importlib.util.spec_from_file_location('bench_edits', os.path.join(HERE, 'edits.py'))
edits = importlib.util.module_from_spec(_spec)
sys.modules[_spec.name] = edits
_spec.loader.exec_module(edits)

OUT = os.path.join(REPO, 'target', 'bs-design')
COUNTS = os.path.join(OUT, 'oracle-counts.jsonl')
CENSUS = os.path.join(OUT, 'census.jsonl')
TREE = os.path.join(OUT, 'tree.json')
DIAGRAMS = os.path.join(OUT, 'diagrams.jsonl')
# What the second machine's oracle cache says: the CPU of each pass.  Read from there by ssh and kept here, as nothing can be written there.
PC_CPU = os.path.join(OUT, 'pc-oracle-cpu.jsonl')
RECORDS = {'laptop': os.path.join(REPO, 'target', 'corpus', 'runs', 'm5-all-2', 'results.jsonl'),
           'pc': os.path.join(REPO, 'target', 'pc', 'corpus', 'pc-all-1.jsonl')}
# The only columns of a corpus record that may be read.  A record also has candidate_ms, candidate_cpu_ms, speedup, outcome and levels, all of them rstex's.
ORACLE_COLUMNS = ('id', 'mode', 'source', 'tier', 'pages', 'oracle_ms', 'oracle_passes', 'oracle_class', 'oracle_errors', 'oracle_key', 'oracle_judged_failure', 'class_mismatch', 'entry_mode')
PRIVATE_PREFIXES = ('user/', 'local/', 'cd-local/')
# Which sources are which.  `real` documents are written to be read, and are the set's population.
POPULATION = {'arxiv': 'real', 'public-stacks': 'real', 'public-books': 'real', 'public-notes': 'real', 'public-manuals': 'real', 'public-knuth': 'real', 'public-latex2e-docs': 'real',
              'public-latex3-docs': 'real', 'ladder': 'ladder', 'local': 'private', 'user': 'private', 'public-latex2e-tests': 'l3build', 'public-latex3-tests': 'l3build',
              'public-package-tests': 'l3build', 'cd': 'diagrams', 'cd-local': 'diagrams-private', 'coverage': 'micro', 'coverage-extra': 'micro'}
# Ends the job when \begin{document} is done: side files read, every \AtBeginDocument run, no page shipped.  So the font map is not read: see MAP_COST in select.py.
PREAMBLE_ONLY = r'\AddToHook{begindocument/end}{\csname @@end\endcsname}'
# The corpus has no real letter, so the kind "letters and small things" also holds the one-line documents of these classes, marked as synthetic.  No other micro document enters a stratum.
LETTER_LIKE = re.compile(r'^coverage/cls/.*(letter|lettre|brief|newlfm|moderncv|europasscv|invoice|exam$)', re.I)


def micro_counted(entry_id):
    """The micro documents the census looks at: the stress cases for a side table, and the letter stand-ins."""
    return entry_id.startswith('coverage/stress/') or LETTER_LIKE.match(entry_id) is not None


def is_private(entry_id):
    return entry_id.startswith(PRIVATE_PREFIXES)


def oracle_columns(record):
    """What may be read of a corpus record.  Everything else stays out, so nothing downstream can see it."""
    return {k: record[k] for k in ORACLE_COLUMNS if k in record}


def read_records(path):
    """{id: {mode: oracle columns}} of a corpus results file."""
    out = {}
    with open(path) as f:
        for line in f:
            r = oracle_columns(json.loads(line))
            out.setdefault(r['id'], {})[r['mode']] = r
    return out


# ------------------------------------------------------------------------------------------------ measure
def oracle_meta(c, e, recorded_key):
    """The cached oracle, for what `run.py` would use if it can, else the one the records name.  Read only: prepare_entry could compile, and could delete another key."""
    meta = c.load_valid_oracle(e)
    if meta is not None:
        meta['fresh'] = True
        return meta
    d = os.path.join(c.oracle_base(e), recorded_key or '')
    if recorded_key and os.path.isfile(os.path.join(d, 'meta.json')):
        meta = c.read_json(os.path.join(d, 'meta.json'))
        meta['dir'] = d
        meta['fresh'] = False
        return meta
    return None


_worker = {}


def worker_init():
    os.nice(10)    # Counts do not care, and other work on this machine may
    c = proflib.corpus()
    c.install_limits()
    entries, _, _ = c.load_manifests(None)
    _worker['c'] = c
    _worker['by_id'] = {e['id']: e for e in entries}


COUNT_EVENTS = [('instructions', 0, 1), ('page-faults', 1, 2)]


def one(job):
    entry_id, what, key = job
    c, e = _worker['c'], _worker['by_id'][entry_id]
    meta = oracle_meta(c, e, key)
    if meta is None or meta.get('observed_class') != 'normal':
        return {'id': entry_id, 'what': what, 'skipped': 'no normal oracle'}
    root = os.path.join(OUT, 'w', secrets.token_hex(8))
    out = os.path.join(root, 'o')
    os.makedirs(out)
    try:
        c.restore_state(meta, out)
        argv = c.tex_argv(e, c.oracle_engine(e), [], out, prelude=PREAMBLE_ONLY if what == 'preamble' else '')
        r = proflib.run_counted(argv, e['cwd'], c.entry_env(e), events=COUNT_EVENTS)
        pdf = os.path.join(out, e['jobname'] + '.pdf')
        return {'id': entry_id, 'what': what, 'key': meta['key'], 'fresh': meta['fresh'], 'rc': r['rc'], 'instructions': r['counters'].get('instructions', {}).get('value'),
                'page_faults': r['counters'].get('page-faults', {}).get('value'), 'maxrss_kb': r['maxrss_kb'], 'cpu_ms': round(r['cpu_ms'], 1),
                'pdf_bytes': os.path.getsize(pdf) if os.path.isfile(pdf) else None}
    finally:
        shutil.rmtree(root, ignore_errors=True)


def measure_cmd(args):
    os.makedirs(OUT, exist_ok=True)
    c = proflib.corpus()
    entries, _, _ = c.load_manifests(None)
    laptop = read_records(RECORDS['laptop'])
    done = set()
    if os.path.exists(COUNTS):
        with open(COUNTS) as f:
            for line in f:
                r = json.loads(line)
                done.add((r['id'], r['what']))
    jobs = []
    pattern = re.compile(args.ids) if args.ids else None
    for e in entries:
        kind = POPULATION.get(e['source'])
        if kind == 'micro' and not micro_counted(e['id']):
            continue
        if e.get('excluded') or e['mode'] not in ('latex', 'plain') or (pattern and not pattern.search(e['id'])):
            continue
        rec = laptop.get(e['id'], {}).get('last-pass', {})
        if rec.get('oracle_class') != 'normal':
            continue
        cost = rec.get('oracle_ms') or 1e9
        if (e['id'], 'total') not in done:
            jobs.append((cost, e['id'], 'total', rec.get('oracle_key')))
        if e['mode'] == 'latex' and (e['id'], 'preamble') not in done and not e.get('prelude'):
            jobs.append((min(cost, 2000), e['id'], 'preamble', rec.get('oracle_key')))
    jobs.sort()
    if args.limit:
        jobs = jobs[:args.limit]
    print(f'{len(jobs)} runs to make, {len(done)} made', flush=True)
    with open(COUNTS, 'a') as sink, cf.ProcessPoolExecutor(max_workers=args.jobs, initializer=worker_init) as pool:
        futures = [pool.submit(one, (i, w, k)) for _, i, w, k in jobs]
        for n, fut in enumerate(cf.as_completed(futures), 1):
            sink.write(json.dumps(fut.result(), sort_keys=True) + '\n')
            sink.flush()
            if n % 200 == 0:
                print(f'{n} / {len(jobs)}', flush=True)
    print('done', flush=True)


# ------------------------------------------------------------------------------------------------ what a document's inputs say
IMAGE_EXT = {'.png', '.jpg', '.jpeg', '.pdf', '.eps', '.mps', '.jbig2', '.jb2'}
TEXT_EXT = {'.tex', '.ltx', '.dtx', '.lagda', '.def', '.inc'}
# A package, by the file the tree holds.  Only basenames of files under the TeX tree are compared to these.
PACKAGE_FILES = {
    'tikz': ('tikz.sty',), 'tikzcd': ('tikz-cd.sty',), 'xy': ('xy.sty', 'xy.tex'), 'pgfplots': ('pgfplots.sty',), 'beamer': ('beamer.cls',), 'microtype': ('microtype.sty',), 'hyperref': ('hyperref.sty',),
    'biblatex': ('biblatex.sty',), 'natbib': ('natbib.sty',), 'cleveref': ('cleveref.sty',), 'longtable': ('longtable.sty', 'supertabular.sty', 'xltabular.sty', 'ltablex.sty', 'tabu.sty', 'ltxtable.sty'),
    'theorem': ('amsthm.sty', 'ntheorem.sty', 'thmtools.sty', 'theorem.sty'), 'amsmath': ('amsmath.sty',), 'babel': ('babel.sty',), 'siunitx': ('siunitx.sty',), 'listings': ('listings.sty',), 'minted': ('minted.sty',),
    'graphicx': ('graphicx.sty',), 'pstricks': ('pstricks.sty', 'pstricks.tex'), 'tcolorbox': ('tcolorbox.sty',), 'algorithm': ('algorithm.sty', 'algorithm2e.sty', 'algorithmicx.sty', 'algpseudocode.sty'),
    'csquotes': ('csquotes.sty',), 'memoir': ('memoir.cls',), 'koma': ('scrartcl.cls', 'scrbook.cls', 'scrreprt.cls', 'scrlttr2.cls'), 'amsart': ('amsart.cls', 'amsbook.cls', 'amsproc.cls'),
    'letter': ('letter.cls', 'scrlttr2.cls', 'newlfm.cls', 'dinbrief.cls', 'lettre.cls', 'isodoc.cls'), 'standalone': ('standalone.cls',),
    'font_lm': ('lmodern.sty',), 'font_times': ('times.sty', 'mathptmx.sty', 'newtxtext.sty', 'newtxmath.sty', 'txfonts.sty', 'tgtermes.sty', 'stix.sty', 'stix2.sty'),
    'font_palatino': ('mathpazo.sty', 'palatino.sty', 'newpxtext.sty', 'newpxmath.sty', 'pxfonts.sty', 'tgpagella.sty'), 'font_libertine': ('libertine.sty', 'libertinus.sty', 'newtxtt.sty'), 'font_t1': ('t1enc.def',),
}
# Constructs counted in a public document's own text.  Bytes only, and nothing is run from them.
SRC_COUNTS = {
    'tikz': re.compile(rb'\\begin\{tikzpicture\}|\\tikz[ \[{\\]'), 'tikzcd': re.compile(rb'\\begin\{tikzcd\}'), 'xy': re.compile(rb'\\xymatrix|\\begin\{xy\}|\\xygraph'),
    'axis': re.compile(rb'\\begin\{(?:semilog[xy]?|loglog|polar|ternary|smithchart)?axis\}|\\begin\{groupplot\}'), 'image': re.compile(rb'\\includegraphics|\\pdfximage'),
    'table': re.compile(rb'\\begin\{(?:tabular[x*]?|longtable\*?|supertabular\*?|xltabular|longtabu|tabu|array)\}'), 'longtable': re.compile(rb'\\begin\{(?:longtable\*?|supertabular\*?|xltabular|longtabu)\}'),
    'cite': re.compile(rb'\\(?:cite|nocite|parencite|textcite|autocite|footcite)\b'), 'index': re.compile(rb'\\index\b'), 'label': re.compile(rb'\\label\b'),
    'math': re.compile(rb'\\begin\{(?:equation|align|gather|multline|eqnarray|displaymath|split|cases)\*?\}|\\\[|\$\$'), 'section': re.compile(rb'\\(?:chapter|section|subsection)\b'),
}
TABLE_BODY = re.compile(rb'\\begin\{(tabular[x*]?|longtable\*?|supertabular\*?|xltabular|longtabu|tabu|array)\}(.*?)\\end\{\1\}', re.S)
ROW = re.compile(rb'\\\\')


def is_tree(path, c):
    return path.startswith(c.TDS_PREFIXES)


def closures(inputs, c):
    """Two pins.  source: the contents of what is not the TeX tree, as a set, so that it means the same on any machine and under any path.  tree: the files of the tree the document read, by path under
    the tree and content, without what a machine generates (formats, font maps), which set.json's `tree` holds once.  Both 20 hex digits."""
    own = sorted({h for p, h in inputs.items() if not is_tree(p, c)})
    tree = sorted(f'{p}\0{h}' for p, h in inputs.items() if p.startswith('/usr/share/texmf-dist/'))
    return hashlib.sha256('\n'.join(own).encode()).hexdigest()[:20], hashlib.sha256('\n'.join(tree).encode()).hexdigest()[:20]


def input_features(inputs, c):
    """From the inputs of a document: which packages by the tree's file names, and how many files of its own by extension.  An own file's name is not kept."""
    names = collections.Counter()
    pfb = tfm = vf = enc = tree_files = 0
    own = collections.Counter()
    own_bytes = 0
    for p in inputs:
        ext = os.path.splitext(p)[1].lower()
        if is_tree(p, c):
            tree_files += 1
            base = os.path.basename(p)
            names[base] += 1
            pfb += ext == '.pfb'
            tfm += ext == '.tfm'
            vf += ext == '.vf'
            enc += ext == '.enc'
        else:
            own[ext if ext in IMAGE_EXT or ext in TEXT_EXT or ext in ('.bbl', '.bib', '.bst', '.ind', '.aux', '.sty', '.cls', '.bcf') else 'other'] += 1
            try:
                own_bytes += os.path.getsize(p)
            except OSError:
                pass
    res = {k: any(n in names for n in v) for k, v in PACKAGE_FILES.items()}
    res.update(tree_files=tree_files, pfb=pfb, tfm=tfm, vf=vf, enc=enc, own_files=sum(own.values()), own_bytes=own_bytes, own_by_ext=dict(sorted(own.items())),
               images=sum(n for e, n in own.items() if e in IMAGE_EXT))
    return res


def source_counts(inputs, c, meta, entry):
    """Of a public document only: constructs in its own text.  A Stacks chapter's .fls lists every chapter of the project, which xr-hyper only looks for, so there only its main file counts."""
    total = collections.Counter()
    size = rows = 0
    only = entry['main'] if group_of(entry['id'], entry['source'])[0] == 'stacks' and entry['id'] != 'stacks/book' else None
    for p in inputs:
        if is_tree(p, c) or os.path.splitext(p)[1].lower() not in TEXT_EXT:
            continue
        if only and os.path.basename(p) != os.path.basename(only):
            continue
        try:
            with open(c.in_this_checkout(p, meta), 'rb') as f:
                data = f.read()
        except OSError:
            continue
        size += len(data)
        text = re.sub(rb'(?m)(?<!\\)%[^\n]*$', b'', data)
        for k, rx in SRC_COUNTS.items():
            total[k] += sum(1 for _ in rx.finditer(text))
        rows += sum(len(ROW.findall(m.group(2))) for m in TABLE_BODY.finditer(text))
    res = {'src_' + k: total[k] for k in SRC_COUNTS}
    res['src_bytes'] = size
    res['src_table_rows'] = rows
    return res


def tools_of(meta):
    """Which of bibtex and makeindex a first compile needs, from the oracle's own steps and side files."""
    ran = {s.get('step') for s in meta.get('steps', []) if s.get('step') != 'tex' and 'skipped' not in s}
    shipped = any(s.get('step') == 'bibtex' and 'skipped' in s and '.bbl' in str(s.get('skipped')) for s in meta.get('steps', []))
    side = meta.get('side_files', {})
    exts = {os.path.splitext(n)[1] for n in side}
    return {'ran_bibtex': 'bibtex' in ran, 'shipped_bbl': shipped, 'ran_makeindex': 'makeindex' in ran, 'has_bbl': '.bbl' in exts, 'has_ind': '.ind' in exts, 'has_bcf': '.bcf' in exts,
            'side_files': len(side)}


def group_of(entry_id, source):
    """(family, field, year).  A family shares its text or its macros, so it is one sample, not many: 15 builds of the HoTT book are one book."""
    p = entry_id.split('/')
    year = None
    if p[0] == 'arxiv':
        field = p[1]
        m = re.match(r'(\d\d)\d\d\.', p[2]) if len(p) > 2 else None
        year = 2000 + int(m.group(1)) if m else None
        return entry_id, field, year
    if p[0] in ('stacks', 'stacks-xr'):
        return 'stacks', 'stacks', None
    if p[0] == 'l3build':
        return '/'.join(p[:3]), 'l3build', None
    if p[0] == 'manuals' and p[1] == 'microtype' and len(p) > 2 and p[2] == 'testsuite':
        return 'manuals/microtype/testsuite', 'l3build', None
    if p[0] == 'docs':
        return '/'.join(p[:3]), 'docs', None
    if p[0] in ('books', 'notes', 'manuals', 'knuth'):
        return '/'.join(p[:2]), p[0], None
    if p[0] in ('local',):
        return '/'.join(p[:2]), 'private', None
    if p[0] == 'user':
        return 'user', 'private', None
    if p[0] in ('cd', 'cd-local'):
        return '/'.join(p[:3]), 'diagrams', None
    return p[0], source, None


# ------------------------------------------------------------------------------------------------ build
def build_cmd(args):
    c = proflib.corpus()
    entries, _, _ = c.load_manifests(None)
    laptop = read_records(RECORDS['laptop'])
    pc = read_records(RECORDS['pc'])
    counts = collections.defaultdict(dict)
    if os.path.exists(COUNTS):
        with open(COUNTS) as f:
            for line in f:
                r = json.loads(line)
                counts[r['id']][r['what']] = r
    pc_cpu = {}
    if os.path.exists(PC_CPU):
        with open(PC_CPU) as f:
            for line in f:
                r = json.loads(line)
                pc_cpu[r['id']] = r['pass_cpu_ms']
    audit = {}
    audit_path = os.path.join(REPO, 'target', 'corpus', 'audit', 'substance.json')
    if os.path.exists(audit_path):
        with open(audit_path) as f:
            audit = json.load(f)['entries']
    tmp = CENSUS + '.tmp'
    written = collections.Counter()
    with open(tmp, 'w') as sink:
        for e in entries:
            kind = POPULATION.get(e['source'])
            private = is_private(e['id'])
            lp = laptop.get(e['id'], {})
            pcr = pc.get(e['id'], {})
            rec = lp.get('last-pass', {})
            row = {'id': e['id'], 'source': e['source'], 'population': kind, 'private': private, 'mode': e['mode'], 'manifest_tier': e['tier'], 'excluded': bool(e.get('excluded')),
                   'class': rec.get('oracle_class'), 'errors': rec.get('oracle_errors'), 'judged_failure': bool(rec.get('oracle_judged_failure')), 'pages': rec.get('pages'), 'passes': rec.get('oracle_passes'),
                   'verdict': (audit.get(e['id']) or {}).get('verdict'), 'on_pc': pcr.get('last-pass', {}).get('oracle_class') == 'normal',
                   'ms_last_laptop': rec.get('oracle_ms'), 'ms_all_laptop': lp.get('from-scratch', {}).get('oracle_ms'),
                   'ms_last_pc': pcr.get('last-pass', {}).get('oracle_ms'), 'ms_all_pc': pcr.get('from-scratch', {}).get('oracle_ms')}
            row['family'], row['field'], row['year'] = group_of(e['id'], e['source'])
            row['cpu_passes_pc'] = pc_cpu.get(e['id'])
            total, preamble = counts[e['id']].get('total', {}), counts[e['id']].get('preamble', {})
            row['instr_total'], row['instr_preamble'] = total.get('instructions'), preamble.get('instructions')
            row['faults_total'], row['maxrss_kb'], row['count_rc'] = total.get('page_faults'), total.get('maxrss_kb'), total.get('rc')
            row['preamble_rc'] = preamble.get('rc')
            if kind != 'micro' or micro_counted(e['id']):
                meta = oracle_meta(c, e, rec.get('oracle_key'))
                if meta is not None:
                    row['oracle_key'], row['oracle_fresh'] = meta['key'], meta['fresh']
                    row['pass_cpu_ms'] = meta.get('pass_cpu_ms')
                    row['pdf_bytes'] = (meta.get('pdf') or {}).get('bytes')
                    log = meta.get('log') or {}
                    row.update(log_bytes=log.get('bytes'), overfull=log.get('overfull'), underfull=log.get('underfull'), converged=meta.get('converged'))
                    row['source_closure'], row['tree_closure'] = closures(meta['inputs'], c)
                    row.update(tools_of(meta))
                    row.update(input_features(meta['inputs'], c))
                    if not private:
                        row.update(source_counts(meta['inputs'], c, meta, e))
                        row['edit_kinds'] = edit_kinds(meta, c, e)
            sink.write(json.dumps(row, sort_keys=True) + '\n')
            written[kind] += 1
    os.replace(tmp, CENSUS)
    with open(TREE, 'w') as f:
        json.dump(tree_info(), f, indent=1, sort_keys=True)
    print(f'{sum(written.values())} rows in {CENSUS}: {dict(written)}')


def edit_kinds(meta, c, e):
    """Which edits of edits.py apply to a public document, in the order its last pass read its own files."""
    job = e['jobname']
    final = os.path.join(meta['dir'], 'final')
    main = e.get('main')
    if not main:
        return []
    _, inputs, _ = c.parse_fls(os.path.join(final, job + '.fls'))
    only = group_of(e['id'], e['source'])[0] == 'stacks' and e['id'] != 'stacks/book'
    files, bib = [], []
    names = list(inputs)
    if main not in names:
        names.insert(0, main)
    for p in names:
        ext = os.path.splitext(p)[1].lower()
        if is_tree(p, c) or ext not in ('.tex', '.ltx', '.dtx') or p.startswith(c.REAL_TARGET + '/corpus/w/') or (only and p != main):
            continue
        try:
            with open(p, 'rb') as f:
                files.append((p, f.read()))
        except OSError:
            pass
    for p in meta['inputs']:
        if p.endswith('.bib') and not is_tree(p, c):
            try:
                with open(p, 'rb') as f:
                    bib.append(f.read())
            except OSError:
                pass
    aux = None
    try:
        with open(os.path.join(final, job + '.aux'), 'rb') as f:
            aux = f.read()
    except OSError:
        pass
    if not any(n == main for n, _ in files):
        return []
    return edits.applicable(files, main, aux, bib)


def file_sha(path):
    try:
        return proflib.sha_file(path)[:20]
    except OSError:
        return None


def lines_of(path):
    try:
        with open(path, 'rb') as f:
            return sum(1 for _ in f)
    except OSError:
        return None


def tree_info():
    """What a result is on, once: the programs and what the tree generates.  A document's own pins are in its row."""
    def out(*argv):
        return subprocess.run(argv, capture_output=True, text=True, check=False).stdout
    maps = out('kpsewhich', 'pdftex.map').strip()
    fmt = out('kpsewhich', '-engine=pdftex', 'pdflatex.fmt').strip()
    official = os.path.join(REPO, 'target', 'texlive-official', 'bin', 'x86_64-linux', 'pdftex')
    return {'oracle_banner': out('pdftex', '--version').splitlines()[0], 'kpathsea': out('pdftex', '--version').splitlines()[1],
            'packages': sorted(line for line in out('pacman', '-Q').splitlines() if line.startswith('texlive')), 'pdftex_sha': file_sha('/usr/bin/pdftex'),
            'pdftex_map_sha': file_sha(maps), 'pdftex_map_lines': lines_of(maps), 'pdflatex_fmt_sha': file_sha(fmt), 'texmf_cnf_sha': file_sha('/usr/share/texmf-dist/web2c/texmf.cnf'),
            'ls_R_lines': lines_of('/usr/share/texmf-dist/ls-R'), 'ls_R_sha': file_sha('/usr/share/texmf-dist/ls-R'), 'official_pdftex_sha': file_sha(official),
            'official_banner': out(official, '--version').splitlines()[0] if os.path.exists(official) else None, 'machine': 'laptop, Ryzen 7 PRO 7730U'}


# ------------------------------------------------------------------------------------------------ diagrams
DIAGRAM = re.compile(rb'\\typeout\{\[\[unit-start ([^\]]+)\]\]\}\n\\setbox\\cdunit=\\hbox\{(.*?)\}\n\\typeout\{\[\[unit ', re.S)
ARROW = re.compile(rb'\\ar(?:row)?\b')
FANCY = re.compile(rb'bend|curve|loop|dash|hook|squiggl|tail|two heads|Rightarrow|phantom|crossing|shift|from=|to=')
STANDALONE = (b"\\documentclass{standalone}\n\\usepackage{amsmath,amssymb}\n\\usepackage{tikz-cd}\n\\begin{document}\n%s\n\\end{document}\n")


def diagram_rows():
    """Each public diagram of tests/corpus/cd: its shape, from its text."""
    c = proflib.corpus()
    with open(os.path.join(REPO, 'tests', 'corpus', 'manifests', 'cd.json')) as f:
        manifest = json.load(f)
    laptop = read_records(RECORDS['laptop'])
    for raw in manifest['entries']:
        entry_id = raw['id']
        rec = laptop.get(entry_id, {}).get('last-pass', {})
        main = c.resolve_path(raw['main'])
        try:
            with open(main, 'rb') as f:
                data = f.read()
        except OSError:
            continue
        for m in DIAGRAM.finditer(data):
            body = m.group(2)
            yield {'entry': entry_id, 'unit': m.group(1).decode(), 'group': entry_id.split('/')[1], 'source_doc': entry_id.split('/', 1)[1], 'class': rec.get('oracle_class'), 'bytes': len(body),
                   'arrows': len(ARROW.findall(body)), 'rows': body.count(b'\\\\') + 1, 'cells': body.count(b'&') + body.count(b'\\\\') + 1, 'fancy': FANCY.search(body) is not None}, body


def one_diagram(job):
    row, body = job
    c = _worker['c']
    root = os.path.join(OUT, 'w', secrets.token_hex(8))
    os.makedirs(root)
    try:
        path = os.path.join(root, 'd.tex')
        with open(path, 'wb') as f:
            f.write(STANDALONE % body)
        env = {k: os.environ[k] for k in ('PATH', 'HOME') if k in os.environ}
        env.update(c.DETERMINISM, **c.NO_MKTEX, **c.HERMETIC)
        r = proflib.run_counted(['pdflatex', '-interaction=batchmode', '-no-shell-escape', 'd.tex'], root, env, events=COUNT_EVENTS)
        row = dict(row, standalone_rc=r['rc'], standalone_pdf=os.path.isfile(os.path.join(root, 'd.pdf')), standalone_instr=r['counters']['instructions']['value'])
        return row
    finally:
        shutil.rmtree(root, ignore_errors=True)


def diagrams_cmd(args):
    """Every public diagram alone under pdflatex with real tikz-cd: which compile, and what they cost.  commute is not asked: what it can handle must not choose the sample."""
    jobs = list(diagram_rows())
    print(f'{len(jobs)} diagrams', flush=True)
    tmp = DIAGRAMS + '.tmp'
    with open(tmp, 'w') as sink, cf.ProcessPoolExecutor(max_workers=args.jobs, initializer=worker_init) as pool:
        for row in pool.map(one_diagram, jobs, chunksize=4):
            sink.write(json.dumps(row, sort_keys=True) + '\n')
    os.replace(tmp, DIAGRAMS)
    print('done', flush=True)


# ------------------------------------------------------------------------------------------------ show
def quantiles(values):
    v = sorted(values)
    if not v:
        return 'none'
    q = lambda f: v[min(len(v) - 1, int(len(v) * f))]    # noqa: E731
    return f'n {len(v)}: min {v[0]:.3g}, p25 {q(0.25):.3g}, median {q(0.5):.3g}, p75 {q(0.75):.3g}, p90 {q(0.9):.3g}, max {v[-1]:.3g}'


def show_cmd(args):
    """The distribution, as tests/bench/README.md quotes it.  Counts and public ids only."""
    with open(CENSUS) as f:
        rows = [json.loads(line) for line in f]
    print(f'{len(rows)} rows.  By population and the oracle\'s class:')
    table = collections.Counter((r['population'], r['class'] or ('excluded' if r['excluded'] else 'none')) for r in rows)
    for k, v in sorted(table.items()):
        print(f'  {k[0]:<18}{k[1]:<16}{v}')
    usable = [r for r in rows if r['class'] == 'normal' and not r['excluded'] and r.get('instr_total')]
    for pop in ('real', 'private', 'ladder', 'l3build', 'diagrams'):
        part = [r for r in usable if r['population'] == pop]
        print(f'\n{pop}: {len(part)} documents with a normal oracle and a count')
        print('  pages:              ', quantiles([r['pages'] or 0 for r in part]))
        print('  passes:             ', dict(sorted(collections.Counter(r['passes'] for r in part).items())))
        print('  Minstr, a last pass:', quantiles([r['instr_total'] / 1e6 for r in part]))
        print('  total, Ginstr:      ', round(sum(r['instr_total'] for r in part) / 1e9))
        share = [(r['instr_preamble'] + 556e6) / r['instr_total'] for r in part if r.get('instr_preamble')]
        print('  preamble share:     ', quantiles([min(1, x) for x in share]))
        print('  PDF MB:             ', quantiles([(r.get('pdf_bytes') or 0) / 1e6 for r in part]))
        print('  peak RSS MB:        ', quantiles([(r.get('maxrss_kb') or 0) / 1e3 for r in part]))
        print('  with TeX errors:    ', sum(1 for r in part if r['errors']), ' on the PC:', sum(1 for r in part if r['on_pc']))
    real = [r for r in usable if r['population'] == 'real']
    print('\nreal documents, by field:')
    for k, v in collections.Counter(r['field'].split('.')[0] if r['field'][:4] in ('math', 'cs.A') or '.' in r['field'] else r['field'] for r in real).most_common():
        print(f'  {k:<12}{v}')
    print('real documents, arXiv by year:', dict(sorted(collections.Counter(r['year'] for r in real if r['year']).items())))
    print('real documents, how many read each:')
    for k in ('hyperref', 'amsmath', 'amsart', 'theorem', 'graphicx', 'font_lm', 'font_times', 'font_palatino', 'microtype', 'babel', 'tikz', 'tikzcd', 'xy', 'pgfplots', 'beamer', 'siunitx', 'listings', 'minted', 'longtable', 'biblatex', 'natbib', 'memoir', 'koma', 'letter', 'pstricks', 'tcolorbox',
              'algorithm', 'cleveref', 'csquotes', 'ran_bibtex', 'shipped_bbl', 'ran_makeindex'):
        print(f'  {k:<16}{sum(1 for r in real if r.get(k))}')
    print('  Type 1 fonts:   ', quantiles([r.get('pfb') or 0 for r in real]))
    print('  own images:     ', quantiles([r.get('images') or 0 for r in real]), ' with any:', sum(1 for r in real if r.get('images')))
    print('  table rows:     ', quantiles([r.get('src_table_rows') or 0 for r in real]))
    print('  own text KB:    ', quantiles([(r.get('src_bytes') or 0) / 1e3 for r in real]))


# ------------------------------------------------------------------------------------------------ main
def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest='cmd', required=True)
    m = sub.add_parser('measure')
    m.add_argument('-j', '--jobs', type=int, default=4)
    m.add_argument('--ids')
    m.add_argument('--limit', type=int)
    sub.add_parser('build')
    sub.add_parser('show')
    d = sub.add_parser('diagrams')
    d.add_argument('-j', '--jobs', type=int, default=4)
    args = ap.parse_args()
    if args.cmd == 'measure':
        measure_cmd(args)
    elif args.cmd == 'build':
        build_cmd(args)
    elif args.cmd == 'diagrams':
        diagrams_cmd(args)
    elif args.cmd == 'show':
        show_cmd(args)


if __name__ == '__main__':
    main()
