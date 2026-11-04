#!/usr/bin/env -S python3 -I
"""baselines.py: what rstex is compared with, measured in instructions.  pdfTeX as Arch builds it against pdfTeX as TeX Live ships it, on one tree, under the same document environment.

    baselines.py formats                         builds pdftex.fmt and pdflatex.fmt for each pdfTeX, in target/bs-design/baseline/, by fmtutil's recipes, counted
    baselines.py sample                          the documents the gap is measured on, by a rule from set.json and set.local.json: target/bs-design/gap-sample.txt
    baselines.py gap --sample FILE [-j 4]        the documents of FILE (one manifest id a line), each engine, each map: target/bs-design/baseline/gap.jsonl
    baselines.py latexmk --sample FILE [-j 4]    a compile from nothing and a rerun with nothing to do, each pdfTeX behind latexmk, counted with its children: latexmk.jsonl
    baselines.py others --sample FILE [-j 4]     lualatex and xelatex, one pass from nothing and a second pass, where they compile at all: others.jsonl
    baselines.py report                          the tables: counts and ids of public documents only

Arch builds pdfTeX with --disable-dump-share, which gives a 64-bit integer and a 16-byte memory word.  The official build has a 32-bit integer and an 8-byte word, and is what most people run.  So a ratio
against Arch's alone may flatter rstex, and this tool is how far.  Both read one tree: Arch's, with its texmf.cnf and its font map; the official binary needs TEXMFCNF, TEXMFROOT and TEXFORMATS for that,
as tests/corpus/run.py's RSTEX_ORACLE_DIR runs did.  Each has formats of its own, built here by one recipe.  Maps: `current` is the tree's pdftex.map; `old` is target/pc/baseline/oldmap, 4,734 lines, by
TEXFONTMAPS, which stands for the map of about 4,400 lines the tree had before three packages were installed.  A document that needs a font only the current map has renders otherwise under `old`, so PDF
sizes are compared and a row where they differ is marked.

Counters are user mode, from outside the process, as tools/prof/proflib.py has them: exact, and blind to the kernel and to what a run costs in cycles.  An 8-byte word may well be faster per instruction, through
the cache, which only a quiet machine can say.  So this answers for instructions and cycles are kept for the coordinator's PC run: `--cycles` makes each document's run count them too, and they mean nothing on a loaded machine.
Private documents appear in output files under target/ only, and `report` prints no private id but those of a local list, and no text.
"""
import os
import sys

# select.py in this directory is not the standard library's select: subprocess needs that one, so this directory must not be on the path.
sys.path[:] = [p for p in sys.path if os.path.abspath(p or os.curdir) != os.path.dirname(os.path.abspath(__file__))]

import argparse  # noqa: E402
import collections  # noqa: E402
import concurrent.futures as cf  # noqa: E402
import hashlib  # noqa: E402
import json  # noqa: E402
import secrets  # noqa: E402
import shlex  # noqa: E402
import shutil  # noqa: E402
import statistics  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, os.path.join(REPO, 'tools', 'prof'))
import proflib  # noqa: E402

OUT = os.path.join(REPO, 'target', 'bs-design', 'baseline')
OFFICIAL_BIN = os.path.join(REPO, 'target', 'texlive-official', 'bin', 'x86_64-linux', 'pdftex')
OLD_MAP = os.path.join(REPO, 'target', 'pc', 'baseline', 'oldmap')
TREE_CNF = {'TEXMFCNF': '/usr/share/texmf-dist/web2c', 'TEXMFROOT': '/usr/share'}
RECIPES = {'pdftex': ['-translate-file=cp227.tcx', '*pdfetex.ini'], 'pdflatex': ['-translate-file=cp227.tcx', '*pdflatex.ini'], 'latex': ['-translate-file=cp227.tcx', '*latex.ini']}
ENGINES = ('arch', 'official')
EVENTS = [('instructions', 0, 1), ('page-faults', 1, 2)]
CYCLE_EVENTS = EVENTS + [('cycles', 0, 0)]


def bin_dir(engine):
    """A directory with pdftex, pdflatex and latex, so that argv[0] says the program as for a user, and resolves to this engine's binary."""
    d = os.path.join(OUT, 'bin-' + engine)
    os.makedirs(d, exist_ok=True)
    target = OFFICIAL_BIN if engine == 'official' else '/usr/bin/pdftex'
    for name in ('pdftex', 'pdflatex', 'latex'):
        link = os.path.join(d, name)
        if not os.path.lexists(link):
            os.symlink(target, link)
    return d


def fmt_dir(engine):
    return os.path.join(OUT, 'fmt-' + engine)


def engine_env(engine, base=None):
    """What differs between the two, on top of a document's environment."""
    env = dict(base or {k: os.environ[k] for k in ('PATH', 'HOME') if k in os.environ})
    env.update(TREE_CNF)
    env['TEXFORMATS'] = '.:' + fmt_dir(engine)
    return env


def formats_cmd(args):
    os.makedirs(OUT, exist_ok=True)
    c = proflib.corpus()
    res = {}
    for engine in ENGINES:
        d = fmt_dir(engine)
        os.makedirs(d, exist_ok=True)
        for name, recipe in RECIPES.items():
            env = engine_env(engine, {k: os.environ[k] for k in ('PATH', 'HOME') if k in os.environ})
            env.update(c.DETERMINISM)
            env.update(c.NO_MKTEX)
            env.update(c.HERMETIC)
            # Formats of the tree alone: a build reads no document.
            exe = os.path.join(bin_dir(engine), 'pdftex')
            argv = [exe, '-ini', '-jobname=' + name, '-progname=' + name] + recipe
            r = proflib.run_counted(argv, d, env, events=EVENTS)
            size = os.path.getsize(os.path.join(d, name + '.fmt'))
            res[f'{engine}/{name}'] = {'rc': r['rc'], 'instructions': r['counters']['instructions']['value'], 'fmt_bytes': size, 'maxrss_kb': r['maxrss_kb']}
            print(f'{engine} {name}: exit {r["rc"]}, {r["counters"]["instructions"]["value"] / 1e9:.3f} G instructions, {size} bytes', flush=True)
    with open(os.path.join(OUT, 'formats.json'), 'w') as f:
        json.dump(res, f, indent=1, sort_keys=True)


# ------------------------------------------------------------------------------------------------ one document
_w = {}


def worker_init():
    os.nice(10)
    c = proflib.corpus()
    c.install_limits()
    entries, _, _ = c.load_manifests(None)
    _w.update(c=c, by_id={e['id']: e for e in entries})


def meta_of(c, e):
    meta = c.load_valid_oracle(e)
    return meta


def argv_of(c, e, engine, out):
    """run.py's argv with this engine's program in front."""
    argv = c.tex_argv(e, c.oracle_engine(e), [], out)
    name = os.path.basename(argv[0])
    argv[0] = os.path.join(bin_dir(engine), name)
    return argv


def run_one(job):
    doc_id, engine, map_name, cycles = job
    c, e = _w['c'], _w['by_id'][doc_id]
    meta = meta_of(c, e)
    if meta is None or meta.get('observed_class') != 'normal':
        return {'id': doc_id, 'engine': engine, 'map': map_name, 'skipped': 'no valid oracle'}
    root = os.path.join(OUT, 'w', secrets.token_hex(8))
    out = os.path.join(root, 'o')
    os.makedirs(out)
    try:
        c.restore_state(meta, out)
        env = engine_env(engine, c.entry_env(e))
        if map_name == 'old':
            env['TEXFONTMAPS'] = OLD_MAP
        r = proflib.run_counted(argv_of(c, e, engine, out), e['cwd'], env, events=CYCLE_EVENTS if cycles else EVENTS)
        pdf = os.path.join(out, e['jobname'] + '.pdf')
        counters = r['counters']
        return {'id': doc_id, 'engine': engine, 'map': map_name, 'rc': r['rc'], 'instructions': counters['instructions']['value'], 'page_faults': counters['page-faults']['value'],
                'cycles': counters.get('cycles', {}).get('value'), 'maxrss_kb': r['maxrss_kb'], 'pdf_bytes': os.path.getsize(pdf) if os.path.isfile(pdf) else None, 'pages_ok': os.path.isfile(pdf)}
    finally:
        shutil.rmtree(root, ignore_errors=True)


def read_sample(path):
    with open(path) as f:
        return [line.split()[0] for line in f if line.strip() and not line.startswith('#')]


def done_set(path, keys):
    done = set()
    if os.path.exists(path):
        with open(path) as f:
            for line in f:
                r = json.loads(line)
                done.add(tuple(r[k] for k in keys))
    return done


def sample_cmd(args):
    """The documents the gap is measured on, by a rule from the set: all of quick, the first of each cell and each feature of standard in a fixed random order, the landmarks of at most a minute on the PC, and the private landmarks."""
    def load(path):
        if not os.path.exists(path):
            return []
        with open(path) as f:
            return json.load(f)['entries']
    public = load(os.path.join(HERE, 'set.json'))
    private = load(os.path.join(HERE, 'set.local.json'))
    chosen = {}
    salt = lambda e: hashlib.sha256(('gap|' + e['id']).encode()).hexdigest()    # noqa: E731
    standard = sorted((e for e in public if 'standard' in e['tiers'] and e['minstr'] < 400000), key=salt)
    for e in public:
        if 'quick' in e['tiers']:
            chosen[e['id']] = e
    cells, feats = set(), set()
    for e in standard:
        cell = (e['kind'], e['size'])
        new_feats = set(e.get('features', ())) - feats
        if cell not in cells or new_feats:
            chosen[e['id']] = e
            cells.add(cell)
            feats |= new_feats
    for e in public + private:
        if e.get('landmark') and e['minstr'] < 400000:
            chosen[e['id']] = e
    ids = sorted(chosen)
    path = os.path.join(REPO, 'target', 'bs-design', 'gap-sample.txt')
    with open(path, 'w') as f:
        f.write('\n'.join(ids) + '\n')
    print(f'{len(ids)} documents in {path}, {sum(1 for i in ids if not i.startswith(("user/", "local/")))} of them public')
    # For latexmk and the other engines, whose runs are from nothing: the public documents of quick, without the regression tests.
    small = sorted(e['id'] for e in public if 'quick' in e['tiers'] and e['kind'] != 'testfile')
    path = os.path.join(REPO, 'target', 'bs-design', 'quick-sample.txt')
    with open(path, 'w') as f:
        f.write('\n'.join(small) + '\n')
    print(f'{len(small)} documents in {path}')


def gap_cmd(args):
    os.makedirs(OUT, exist_ok=True)
    path = os.path.join(OUT, 'gap.jsonl')
    done = done_set(path, ('id', 'engine', 'map'))
    jobs = [(d, en, m, args.cycles) for d in read_sample(args.sample) for en in ENGINES for m in ('current', 'old') if (d, en, m) not in done]
    print(f'{len(jobs)} runs to make', flush=True)
    with open(path, 'a') as sink, cf.ProcessPoolExecutor(max_workers=args.jobs, initializer=worker_init) as pool:
        for r in pool.map(run_one, jobs):
            sink.write(json.dumps(r, sort_keys=True) + '\n')
            sink.flush()
    print('done', flush=True)


# ------------------------------------------------------------------------------------------------ latexmk
def run_latexmk(job):
    doc_id, engine = job
    c, e = _w['c'], _w['by_id'][doc_id]
    root = os.path.join(OUT, 'w', secrets.token_hex(8))
    out = os.path.join(root, 'o')
    os.makedirs(out)
    try:
        env = engine_env(engine, c.entry_env(e))
        env.pop('openin_any', None)    # bibtex under latexmk reads an absolute .bib
        env.pop('openout_any', None)
        exe = os.path.join(bin_dir(engine), 'pdflatex')
        # What latexmk decides by itself, from the .aux and the .fls: bibtex, biber and makeindex run if it says so.
        argv = ['latexmk', '-norc', '-pdf', '-pdflatex=' + shlex.quote(exe) + ' -interaction=batchmode -no-shell-escape %O %S', '-outdir=' + out, e['main_rel']]
        first = proflib.run_counted(argv, e['cwd'], env, events=EVENTS)
        again = proflib.run_counted(argv, e['cwd'], env, events=EVENTS)
        pdf = os.path.join(out, e['jobname'] + '.pdf')
        return {'id': doc_id, 'engine': engine, 'first_rc': first['rc'], 'first_instructions': first['counters']['instructions']['value'], 'again_rc': again['rc'],
                'again_instructions': again['counters']['instructions']['value'], 'pdf': os.path.isfile(pdf)}
    finally:
        shutil.rmtree(root, ignore_errors=True)


def latexmk_cmd(args):
    path = os.path.join(OUT, 'latexmk.jsonl')
    done = done_set(path, ('id', 'engine'))
    jobs = [(d, en) for d in read_sample(args.sample) for en in ENGINES if (d, en) not in done]
    print(f'{len(jobs)} runs to make', flush=True)
    with open(path, 'a') as sink, cf.ProcessPoolExecutor(max_workers=args.jobs, initializer=worker_init) as pool:
        for r in pool.map(run_latexmk, jobs):
            sink.write(json.dumps(r, sort_keys=True) + '\n')
            sink.flush()


# ------------------------------------------------------------------------------------------------ lualatex and xelatex
def run_other(job):
    doc_id, program = job
    c, e = _w['c'], _w['by_id'][doc_id]
    if e['mode'] != 'latex':
        return {'id': doc_id, 'program': program, 'skipped': 'not LaTeX'}
    root = os.path.join(OUT, 'w', secrets.token_hex(8))
    out = os.path.join(root, 'o')
    os.makedirs(out)
    try:
        env = c.entry_env(e)
        var = os.path.join(OUT, 'var-' + program)    # luaotfload and fontconfig keep a cache: its first fill is not what a document costs
        os.makedirs(var, exist_ok=True)
        env['TEXMFVAR'] = var
        env['HOME'] = var
        res = {'id': doc_id, 'program': program}
        for k in ('pdflatex', program):
            for i in (1, 2):
                argv = [k, '-interaction=batchmode', '-no-shell-escape', '-output-directory=' + out + '-' + k]
                os.makedirs(out + '-' + k, exist_ok=True)
                argv.append(e['main_rel'])
                the_env = dict(env) if k != 'pdflatex' else dict(c.entry_env(e))
                r = proflib.run_counted(argv, e['cwd'], the_env, events=EVENTS)
                res[f'{k}_pass{i}'] = r['counters']['instructions']['value']
                res[f'{k}_rc{i}'] = r['rc']
            res[f'{k}_pdf'] = os.path.isfile(os.path.join(out + '-' + k, e['jobname'] + '.pdf'))
        return res
    finally:
        shutil.rmtree(root, ignore_errors=True)


def others_cmd(args):
    path = os.path.join(OUT, 'others.jsonl')
    done = done_set(path, ('id', 'program'))
    jobs = [(d, p) for d in read_sample(args.sample) for p in ('lualatex', 'xelatex') if (d, p) not in done]
    print(f'{len(jobs)} runs to make', flush=True)
    with open(path, 'a') as sink, cf.ProcessPoolExecutor(max_workers=args.jobs, initializer=worker_init) as pool:
        for r in pool.map(run_other, jobs):
            sink.write(json.dumps(r, sort_keys=True) + '\n')
            sink.flush()


# ------------------------------------------------------------------------------------------------ report
def geo(values):
    values = [v for v in values if v > 0]
    return statistics.geometric_mean(values) if values else None


def spread(values):
    v = sorted(values)
    q = lambda f: v[min(len(v) - 1, int(len(v) * f))]    # noqa: E731
    return f'n {len(v):<3} geomean {geo(v):.4f}  min {v[0]:.4f}  p25 {q(0.25):.4f}  median {q(0.5):.4f}  p75 {q(0.75):.4f}  max {v[-1]:.4f}'


def set_rows():
    res = {}
    for name in ('set.json', 'set.local.json'):
        path = os.path.join(HERE, name)
        if os.path.exists(path):
            with open(path) as f:
                res.update({e['id']: e for e in json.load(f)['entries']})
    return res


def report_cmd(args):
    """Counts only, and ids of public documents only."""
    private = lambda i: i.startswith(('user/', 'local/', 'cd-local/'))    # noqa: E731
    known = set_rows()
    with open(os.path.join(OUT, 'formats.json')) as f:
        fmts = json.load(f)
    print('pdflatex.fmt, G instructions: ' + ', '.join(f'{k} {v["instructions"] / 1e9:.3f}' for k, v in sorted(fmts.items()) if k.endswith('/pdflatex')))
    print(f'  official / arch {fmts["official/pdflatex"]["instructions"] / fmts["arch/pdflatex"]["instructions"]:.4f}; format bytes {fmts["arch/pdflatex"]["fmt_bytes"]} and {fmts["official/pdflatex"]["fmt_bytes"]}')
    rows = collections.defaultdict(dict)
    with open(os.path.join(OUT, 'gap.jsonl')) as f:
        for line in f:
            r = json.loads(line)
            if 'instructions' in r:
                rows[r['id']][(r['engine'], r['map'])] = r
    table = [(doc, by) for doc, by in sorted(rows.items()) if len(by) == 4]
    print(f'{len(table)} documents with all four runs, {sum(1 for d, _ in table if private(d))} of them private')
    pair = lambda by, m: by[('official', m)]['instructions'] / by[('arch', m)]['instructions']    # noqa: E731
    print('official / arch, current map: ' + spread([pair(by, 'current') for _, by in table]))
    print('official / arch, old map:     ' + spread([pair(by, 'old') for _, by in table]))
    print('old map / current, arch:      ' + spread([by[('arch', 'old')]['instructions'] / by[('arch', 'current')]['instructions'] for _, by in table]))
    print('old map / current, official:  ' + spread([by[('official', 'old')]['instructions'] / by[('official', 'current')]['instructions'] for _, by in table]))
    print('faults, official / arch:      ' + spread([by[('official', 'current')]['page_faults'] / by[('arch', 'current')]['page_faults'] for _, by in table if by[('arch', 'current')]['page_faults']]))
    print('peak RSS, official / arch:    ' + spread([by[('official', 'current')]['maxrss_kb'] / by[('arch', 'current')]['maxrss_kb'] for _, by in table]))
    diff = sorted(by[('arch', 'current')]['instructions'] - by[('official', 'current')]['instructions'] for _, by in table)
    print(f'arch less official, current map, Minstr: min {diff[0] / 1e6:.0f}, median {diff[len(diff) // 2] / 1e6:.0f}, max {diff[-1] / 1e6:.0f}')
    for what in ('size', 'kind'):
        groups = collections.defaultdict(list)
        for doc, by in table:
            if doc in known:
                groups[known[doc][what]].append(pair(by, 'current'))
        print(f'official / arch, current map, by {what}:')
        for k, v in sorted(groups.items()):
            print(f'  {k:<16}{spread(v)}')
    features = collections.defaultdict(list)
    for doc, by in table:
        if doc in known:
            for f in known[doc].get('features', ()):
                features[f].append(pair(by, 'current'))
    print('official / arch, current map, by feature:')
    for k, v in sorted(features.items()):
        print(f'  {k:<22}{spread(v)}')
    bad = sum(1 for _, by in table for r in by.values() if r['rc'] != 0)
    print(f'{bad} runs of {4 * len(table)} had a nonzero exit status (a document with TeX errors does, on every side)')
    moved = [d for d, by in table if by[('arch', 'current')].get('pdf_bytes') != by[('arch', 'old')].get('pdf_bytes')]
    print(f'{len(moved)} of {len(table)} documents have another PDF size under the old map: ' + ' '.join(d for d in moved if not private(d)))
    far = [d for d, by in table if abs((by[('arch', 'current')].get('pdf_bytes') or 0) - (by[('official', 'current')].get('pdf_bytes') or 0)) > 256]
    print(f'{len(far)} of {len(table)} have PDFs more than 256 bytes apart between the two pdfTeX: ' + ' '.join(d for d in far if not private(d)))
    print(f'{"public document":<56}{"Minstr":>9}{"off/arch":>9}{"old map":>9}{"off/arch":>9}')
    for doc, by in table:
        if private(doc):
            continue
        ac, oc, ao, oo = by[('arch', 'current')], by[('official', 'current')], by[('arch', 'old')], by[('official', 'old')]
        print(f'{doc[:54]:<56}{ac["instructions"] / 1e6:>9.0f}{oc["instructions"] / ac["instructions"]:>9.4f}{ao["instructions"] / 1e6:>9.0f}{oo["instructions"] / ao["instructions"]:>9.4f}')
    path = os.path.join(OUT, 'latexmk.jsonl')
    if os.path.exists(path):
        mk = collections.defaultdict(dict)
        with open(path) as f:
            for line in f:
                r = json.loads(line)
                mk[r['id']][r['engine']] = r
        both = {d: v for d, v in mk.items() if len(v) == 2}
        print(f'latexmk: {len(both)} documents, {sum(1 for v in both.values() if v["arch"]["first_rc"] == 0)} of them with exit 0 under Arch\'s')
        print('  from nothing, official / arch: ' + spread([v['official']['first_instructions'] / v['arch']['first_instructions'] for v in both.values()]))
        again = sorted(v['arch']['again_instructions'] / 1e6 for v in both.values())
        print(f'  rerun with nothing to do, Minstr under Arch\'s: min {again[0]:.0f}, median {again[len(again) // 2]:.0f}, max {again[-1]:.0f}')
    path = os.path.join(OUT, 'others.jsonl')
    if os.path.exists(path):
        print('lualatex and xelatex against pdflatex, the second pass of each from nothing.  Their output differs: other fonts and other line breaks.  This is not the same work.')
        rows_o = collections.defaultdict(list)
        total = collections.Counter()
        with open(path) as f:
            for line in f:
                r = json.loads(line)
                if 'skipped' in r:
                    continue
                prog = r['program']
                total[prog] += 1
                if r[f'{prog}_rc2'] == 0 and r[f'{prog}_pdf'] and r['pdflatex_rc2'] == 0:
                    rows_o[prog].append(r[f'{prog}_pass2'] / r['pdflatex_pass2'])
        for prog, v in sorted(rows_o.items()):
            print(f'  {prog}: exit 0 with a PDF on {len(v)} of {total[prog]}: {spread(v)}')


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest='cmd', required=True)
    sub.add_parser('formats')
    sub.add_parser('sample')
    for name in ('gap', 'latexmk', 'others'):
        p = sub.add_parser(name)
        p.add_argument('--sample', required=True)
        p.add_argument('-j', '--jobs', type=int, default=4)
        if name == 'gap':
            p.add_argument('--cycles', action='store_true')
    sub.add_parser('report')
    args = ap.parse_args()
    {'formats': formats_cmd, 'sample': sample_cmd, 'gap': gap_cmd, 'latexmk': latexmk_cmd, 'others': others_cmd, 'report': report_cmd}[args.cmd](args)


if __name__ == '__main__':
    main()
