#!/usr/bin/env -S python3 -I
"""select.py: the rule that picks the benchmark set.  tests/bench/README.md says why it is this rule.

    select.py [--census FILE] [--diagrams FILE] [--tree FILE] [--landmarks FILE] [--out DIR]    writes set.json and, if there are private rows and a landmark file, set.local.json
    select.py --show                                                                            prints what a set holds: counts and public ids only
    select.py --check                                                                           whether each pin of the sets still holds, against the census: counts, and public ids only

It reads the census of tests/bench/census.py, which holds the oracle's side only, and a row with anything of rstex's in it stops the program.  It reads no clock, and a seed is fixed below, so the same census gives
the same bytes.  The public part does not depend on the private part: without a private row, set.json is the same file.  Nothing here looks at what any engine gains.

Order of the rule, each step on what the last one left:
    1 Eligible: the oracle's class is normal, it did not fail on a test judged by its failure, a page came out (a regression test may have none), audit.py finds it neither vacuous nor a duplicate, and its oracle is current.
    2 Landmarks are in, by name, each with a reason.
    3 The hold-out: whole families that hold no landmark and no document that training or a tool names, up to a share of each stratum.  Nothing else ever comes from those families.
    4 standard: a proportional draw from each cell of kind and size, in a fixed random order, at most so many from one family, and a floor for each feature.
    5 quick: a part of standard.  full: everything left.
    6 Weights, so that a pooled figure gives each cell its share of the population.
"""
import os
import sys

# select.py in this directory is not the standard library's select: subprocess needs that one, so this directory must not be on the path.
sys.path[:] = [p for p in sys.path if os.path.abspath(p or os.curdir) != os.path.dirname(os.path.abspath(__file__))]

import argparse  # noqa: E402
import ast  # noqa: E402
import collections  # noqa: E402
import hashlib  # noqa: E402
import json  # noqa: E402
import math  # noqa: E402
import statistics  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
OUT = os.path.join(REPO, 'target', 'bs-design')

SEED = 'rstex-bench-set-1'
# Anything rstex produced.  A census row with one of these is refused, and a name holding one of the words too.
FORBIDDEN = ('candidate_ms', 'candidate_cpu_ms', 'candidate_rc', 'candidate_signal', 'speedup', 'outcome', 'levels', 'first_divergence', 'differs_as', 'known_reason', 'artifacts', 'inst_note', 'log_note', 'retried')
FORBIDDEN_WORDS = ('candidate', 'speedup', 'rstex', 'commute', 'eligib')

# ------------------------------------------------------------------------------------------------ strata
SIZES = (('one-page', 1), ('note', 9), ('paper', 39), ('long-paper', 119), ('book', 599), ('very-long', 10 ** 9))
KINDS = ('math', 'cs', 'physics-other', 'beamer', 'manual', 'book', 'small', 'testfile')
FEATURES = ('heavy-tikz', 'tikz-cd', 'xy-pic', 'pgfplots', 'microtype', 'biblatex', 'bibtex', 'index', 'many-images', 'huge-tables', 'mostly-text', 'nearly-all-preamble', 'plain-tex')
# Thresholds on the census, all set before any result of any engine was looked at.
T_TIKZ, T_TIKZCD, T_XY, T_AXIS, T_INDEX, T_IMAGES, T_ROWS, T_LONG_ROWS = 10, 5, 5, 1, 10, 10, 200, 50
T_PREAMBLE, T_TEXT_QUANTILE, T_TEXT_PAGES = 0.9, 0.25, 5

# A family past this many documents counts as this many: it shares its macros, and often its text, so it is not that many samples.  It gives 233 Stacks chapters the weight of 12 papers.
FAMILY_MASS = 12
STANDARD_N = 200        # real documents drawn with the landmarks, before the floors: what fits an hour, see `estimate`
TESTFILES_STANDARD, TESTFILES_QUICK, TESTFILES_HOLDOUT = 16, 2, 8    # regression tests, a small stratum, never pooled with real documents
CELL_FLOOR = 2          # a cell with documents has at least this many, or all it has
FAMILY_CAP = 0.2        # of a cell in standard, at most this share, or two, from one family
FEATURE_FLOOR = 10      # a feature has at least this many in standard, or all it has
HOLDOUT_SHARE = 0.22    # of each cell's mass, in whole families kept apart
HOLDOUT_N = 110         # real documents drawn from them, with the same proportions
HOLDOUT_FEATURE_FLOOR = 3
# A family that alone supplies a stratum, and whose documents are separate works, is two families, or both sides of the hold-out could not have that stratum: the slides of one author, the letter stand-ins.
SPLIT_FAMILIES = ('notes/internal-methods', 'coverage')
PRIVATE_STANDARD_N, PRIVATE_HOLDOUT_N, PRIVATE_MAX_S = 30, 16, 60.0    # of the owner's documents that the PC holds, by the same rule and reported apart
QUICK_N = 24            # with the landmarks that are small
QUICK_MAX_S = 4.0       # no document longer than this on the PC in quick, but one
EDIT_QUOTA = {'one-page': 3, 'note': 7, 'paper': 12, 'long-paper': 8, 'book': 4, 'very-long': 0}
EDIT_MAX_S = 12.0
PIXEL_QUOTA = {'one-page': 2, 'note': 2, 'paper': 3, 'long-paper': 3, 'book': 2, 'very-long': 0}
DIAGRAMS_STANDARD, DIAGRAMS_QUICK, DIAGRAMS_HOLDOUT = 200, 20, 100
ARROW_BUCKETS = ((1, '1'), (3, '2-3'), (7, '4-7'), (10 ** 6, '8+'))
DIAGRAM_SOURCES = ('arxiv', 'notes', 'manuals', 'books')    # written by people for reading.  tests-cd and the ladder were written for the native path, so they are in `full` alone

# Named documents with why.  These are public; the private ones are ids in landmarks.local.json, which is git-ignored.
LANDMARKS = {
    'stacks/book': 'largest document in the corpus, 7,710 pages; xy-pic throughout',
    'docs/latex3/l3kernel/source3.tex': 'the LaTeX3 sources, 1,780 pages; the most expl3',
    'docs/latex2e/base/source2e.tex': 'the LaTeX kernel sources, 1,322 pages',
    'manuals/pgf/doc/generic/pgf/pgfmanual': 'the TikZ manual, 1,180 pages; the heaviest PGF in the corpus',
    'books/napkin/Napkin': 'a long book with many figures, 1,014 pages',
    'books/hott/hott-a4': 'the HoTT book, the public book training uses, so it is marked',
    'manuals/koma-script/doc/scrguide-en': 'the KOMA-Script guide, 500 pages; a real table-heavy manual',
    'manuals/memoir/doc-src/memman': 'the memoir manual, 533 pages; the most table rows in the corpus',
    'manuals/hyperref/hyperref': 'a long .dtx: the hyperref sources',
    'manuals/beamer/doc/beameruserguide': 'the beamer guide, 219 pages',
    'manuals/tikz-cd/tikz-cd-doc': 'the tikz-cd manual, which training uses, so it is marked',
    'manuals/lshort/book/src/lshort': 'what a new user reads and compiles',
    'manuals/siunitx/siunitx': 'an expl3 package at length, with tables',
    'knuth/woven/tex': 'tex.web, woven, 536 pages under plain TeX: no LaTeX at all',
    'arxiv/math.CT/2503.22699v1': 'the longest paper in the arXiv sample, 345 pages',
    'arxiv/math.CT/1904.08083v1': 'the most TikZ pictures of any paper in the sample',
    'ladder/04-latex-hello': 'the floor under LaTeX: a target of DESIGN section 2; training uses it',
    'ladder/00-plain-hello': 'the floor under plain TeX: a target of DESIGN section 2; training uses it',
    'ladder/15-tikzcd-many': 'many diagrams in one page set: a target of DESIGN section 2; training uses it',
    'ladder/03-plain-texweb': 'a target of DESIGN section 2; training uses it',
}
# The same text as another document, but for a line: found by comparing word shingles of all public main files, which also says that no other row of the set is so close to a trained document.
SAME_TEXT = {'knuth/woven/tex': 'ladder/03-plain-texweb'}
# Public ids that tracked tools and documents named by the time this rule was fixed, in tools/ and docs/ of dff097f and 8e83f07: performance work may have looked at them.  test_select.py scans again, so a later tool that names
# a document of the hold-out fails the test.
TOOLS_NAMED = (
    'arxiv/math.AG/1912.07119v3', 'arxiv/math.AG/2007.09646v1', 'arxiv/cs.NI/1908.01405v1', 'books/hott/hott-a4', 'docs/latex2e/required--graphics/grfguide.tex', 'docs/latex2e/required--latex-lab/latex-lab-block.dtx',
    'l3build/latex2e/base--testfiles-lthooks/github-1078', 'l3build/latex2e/base--testfiles-search/tlb-metadata-000', 'l3build/latex3/l3kernel--testfiles/m3benchmark001',
    'manuals/koma-script/doc/scrguide-en', 'manuals/pgf/doc/generic/pgf/pgfmanual', 'manuals/tikz-cd/tikz-cd-doc',
)


# ------------------------------------------------------------------------------------------------ small things
def die(message):
    sys.stderr.write(f'select.py: {message}\n')
    sys.exit(2)


def hash_of(*parts):
    return hashlib.sha256('\0'.join((SEED,) + parts).encode()).hexdigest()


def in_order(items, salt, key=lambda r: r['id']):
    """A fixed random order."""
    return sorted(items, key=lambda r: hash_of(salt, key(r)))


def forbidden_in(row):
    return [k for k in row if k in FORBIDDEN or any(w in k.lower() for w in FORBIDDEN_WORDS)]


def read_jsonl(path):
    with open(path) as f:
        return [json.loads(line) for line in f]


def largest_remainder(want, total):
    """Whole numbers that add to `total`, each at least its `floor`.  `want` is {key: (share, floor, most)}, and a key's own hash breaks ties, so the answer does not depend on the order of the input."""
    out = {k: min(f, m) for k, (_, f, m) in want.items()}
    left = total - sum(out.values())
    # Hand out one at a time to the key furthest under its share: D'Hondt, which is exact, simple, and gives the same result on any machine.
    while left > 0:
        best, best_score = None, -1.0
        for k in sorted(want):
            share, _, most = want[k]
            if out[k] >= most:
                continue
            score = share / (out[k] + 1)
            if score > best_score:
                best, best_score = k, score
        if best is None:
            break
        out[best] += 1
        left -= 1
    return out


# ------------------------------------------------------------------------------------------------ what a row is
def size_of(pages):
    for name, most in SIZES:
        if (pages or 0) <= most:
            return name
    return SIZES[-1][0]


def kind_of(r):
    if r.get('private'):
        return 'private'
    if r['field'] == 'l3build':
        return 'testfile'
    if r.get('beamer'):
        return 'beamer'
    if r.get('letter') or r.get('population') == 'micro' or (r.get('pages') or 0) <= 2:
        return 'small'
    f = r['field']
    if f.startswith('math.') or f in ('stacks', 'notes'):
        return 'book' if r['id'] == 'stacks/book' else 'math'
    if f.startswith('cs.'):
        return 'cs'
    if f.startswith(('hep-', 'stat.', 'q-bio.', 'physics', 'astro', 'cond-mat', 'quant')):
        return 'physics-other'
    if f == 'books' or r['id'] in ('knuth/tex/texbook', 'knuth/mf/mfbook'):
        return 'book'
    return 'manual'


def features_of(r, text_limit):
    n = lambda k: r.get(k) or 0    # noqa: E731
    f = []
    if r.get('tikz') and n('src_tikz') >= T_TIKZ:
        f.append('heavy-tikz')
    if r.get('tikzcd') and n('src_tikzcd') >= T_TIKZCD:
        f.append('tikz-cd')
    if r.get('xy') and n('src_xy') >= T_XY:
        f.append('xy-pic')
    if r.get('pgfplots') and n('src_axis') >= T_AXIS:
        f.append('pgfplots')
    if r.get('microtype'):
        f.append('microtype')
    if r.get('biblatex'):
        f.append('biblatex')
    if r.get('ran_bibtex'):
        f.append('bibtex')
    if r.get('ran_makeindex') and n('src_index') >= T_INDEX:
        f.append('index')
    if n('images') >= T_IMAGES:
        f.append('many-images')
    if n('src_table_rows') >= T_ROWS or (n('src_longtable') >= 1 and n('src_table_rows') >= T_LONG_ROWS):
        f.append('huge-tables')
    if r.get('mode') == 'plain':
        f.append('plain-tex')
    share = r.get('preamble_share')
    if share is not None and share >= T_PREAMBLE:
        f.append('nearly-all-preamble')
    # A document whose preamble alone is this much of a pass is what a checkpoint gains most on, so what the rule calls it does not depend on the engine: only on the oracle's two counts.
    heavy = {'heavy-tikz', 'tikz-cd', 'xy-pic', 'pgfplots', 'many-images', 'huge-tables'}
    body = r.get('body_per_page')
    if body is not None and text_limit is not None and body <= text_limit and (r.get('pages') or 0) >= T_TEXT_PAGES and not heavy & set(f):
        f.append('mostly-text')
    return f


def eligible(r):
    """Step 1.  The oracle's own class decides: a document pdflatex fails on is not comparable.  Nothing here can see what rstex did, so one it fails on stays, and counts against it."""
    if r.get('excluded') or r.get('class') != 'normal' or r.get('judged_failure') or r.get('verdict') in ('vacuous', 'duplicate'):
        return False
    if r.get('instr_total') is None or r.get('source_closure') is None or not r.get('oracle_fresh'):
        return False
    if (r.get('pages') or 0) < 1 and r.get('field') != 'l3build':
        return False
    return r.get('count_rc') in (0, 1)


def startup_floor(rows):
    """What the first page costs on top of a run that ships none: mostly reading the font map.  From the oracle's two runs of the smallest LaTeX document."""
    for r in rows:
        if r['id'] == 'ladder/04-latex-hello' and r.get('instr_total') and r.get('instr_preamble'):
            return r['instr_total'] - r['instr_preamble']
    return 0


def derive(rows):
    """Adds what the rule uses.  Copies, so the caller's rows stay as read."""
    floor = startup_floor(rows)
    out = []
    for r in rows:
        r = dict(r)
        total, pre = r.get('instr_total'), r.get('instr_preamble')
        if total and pre:
            r['preamble_share'] = min(1.0, (pre + floor) / total)
            r['body_per_page'] = max(0, total - pre - floor) / max(1, r.get('pages') or 1)
        r['kind'], r['size'] = kind_of(r), size_of(r.get('pages'))
        r['family'] = split_family(r['family'], r['id'])
        out.append(r)
    pool = sorted(r['body_per_page'] for r in out if r.get('population') == 'real' and r.get('body_per_page') is not None and (r.get('pages') or 0) >= T_TEXT_PAGES)
    text_limit = pool[int(len(pool) * T_TEXT_QUANTILE)] if pool else None
    for r in out:
        r['features'] = features_of(r, text_limit)
    return out, floor, text_limit


def split_family(family, doc_id):
    if family in SPLIT_FAMILIES:
        return f'{family}#{int(hash_of("split", doc_id), 16) % 2}'
    return family


def cell_of(r):
    return f'{r["kind"]}/{r["size"]}'


def family_mass(rows):
    sizes = collections.Counter(r['family'] for r in rows)
    return {f: min(1.0, FAMILY_MASS / n) for f, n in sizes.items()}


# ------------------------------------------------------------------------------------------------ what training touches
def pgo_lists():
    """The training list and the short names, by reading tools/pgo.py and tools/prof/proflib.py as text.  Importing them would run a lot for three lists."""
    def constants(path, names):
        with open(path) as f:
            tree = ast.parse(f.read())
        res = {}
        for node in tree.body:
            if isinstance(node, ast.Assign) and isinstance(node.targets[0], ast.Name) and node.targets[0].id in names:
                res[node.targets[0].id] = ast.literal_eval(node.value)
        return res
    pgo = constants(os.path.join(REPO, 'tools', 'pgo.py'), {'TRAIN_ALWAYS', 'TRAIN_PUBLIC_OPTIONAL', 'TRAIN_PRIVATE', 'TRAIN_BOTH_WAYS', 'BENCH_DOCS'})
    named = constants(os.path.join(REPO, 'tools', 'prof', 'proflib.py'), {'NAMED'})['NAMED']
    return pgo, named


def training_ids(all_ids):
    """Every manifest id tools/pgo.py trains on, private ones among them, expanded, with what is the same document under another id and a line apart, which a closure cannot see.  They are not printed."""
    pgo, named = pgo_lists()
    res = set()
    for key in ('TRAIN_ALWAYS', 'TRAIN_PUBLIC_OPTIONAL', 'TRAIN_PRIVATE', 'TRAIN_BOTH_WAYS'):
        for name in pgo.get(key, ()):
            if name.endswith('/*'):
                res |= {i for i in all_ids if i.startswith(name[:-1])}
            else:
                res.add(named.get(name, name))
    return res | {twin for twin, of in SAME_TEXT.items() if of in res}


# ------------------------------------------------------------------------------------------------ the draw
def choose_holdout(pop, touched_families, mass, touched_rows=()):
    """Step 3.  Whole families, in a fixed random order, each taken if a cell of it is under its share and none goes far over.  Returns the families."""
    by_family = collections.defaultdict(list)
    for r in pop:
        by_family[r['family']].append(r)
    cell_mass = collections.Counter()
    for r in pop:
        cell_mass[cell_of(r)] += mass[r['family']]
    want = {c: HOLDOUT_SHARE * m for c, m in cell_mass.items()}
    have = collections.Counter()
    chosen = set()
    for fam in in_order(sorted(by_family), 'holdout-family', key=lambda f: f):
        if fam in touched_families:
            continue
        adds = collections.Counter()
        for r in by_family[fam]:
            adds[cell_of(r)] += mass[fam]
        helps = any(have[c] < want[c] for c in adds)
        # A cell of one or two documents may go empty of the rest: keep at least half of each cell's mass out of the hold-out.
        fits = all(have[c] + adds[c] <= max(want[c] * 1.5, 1.0) and have[c] + adds[c] <= cell_mass[c] * 0.5 + 1e-9 for c in adds)
        if helps and fits:
            chosen.add(fam)
            have.update(adds)
    # A kind or a size with nothing yet takes the first family that has it, in the same order, if some of that kind or size is left for the tiers.
    for dim in ('kind', 'size'):
        values = sorted({r[dim] for r in pop})
        for value in values:
            if any(r[dim] == value and r['family'] in chosen for r in pop):
                continue
            for fam in in_order(sorted(by_family), 'holdout-family', key=lambda f: f):
                if fam in touched_families or fam in chosen or not any(r[dim] == value for r in by_family[fam]):
                    continue
                if any(r[dim] == value and r['family'] != fam and r['family'] not in chosen for r in pop) or any(r[dim] == value for r in touched_rows):
                    chosen.add(fam)
                    break
    return chosen


def draw(pool, n, salt, cap_share=FAMILY_CAP, family_counts=None):
    """`n` documents of a cell in a fixed random order, at most so many of a family while there are others to take."""
    order = in_order(pool, salt)
    cap = max(2, math.ceil(n * cap_share))
    got, count = [], collections.Counter(family_counts or {})
    for r in order:
        if len(got) >= n:
            break
        if count[r['family']] < cap:
            got.append(r)
            count[r['family']] += 1
    if len(got) < n:
        have = {r['id'] for r in got}
        got += [r for r in order if r['id'] not in have][:n - len(got)]
    return got


def allocate(pop, mass, total, floor=CELL_FLOOR):
    cells = collections.defaultdict(list)
    for r in pop:
        cells[cell_of(r)].append(r)
    want = {c: (sum(mass[r['family']] for r in rs), floor, len(rs)) for c, rs in cells.items()}
    return largest_remainder(want, total), cells


def pick_standard(pop, mass, landmark_rows, n_total, floors=True):
    """Step 4.  Returns the chosen rows by id, and the ids only a floor brought in."""
    alloc, cells = allocate(pop, mass, n_total)
    chosen = {r['id']: r for r in landmark_rows}
    for c in sorted(cells):
        already = [r for r in cells[c] if r['id'] in chosen]
        need = max(0, alloc[c] - len(already))
        rest = [r for r in cells[c] if r['id'] not in chosen]
        for r in draw(rest, need, 'standard|' + c, family_counts=collections.Counter(x['family'] for x in already)):
            chosen[r['id']] = r
    drawn = set(chosen)
    # The floors: a feature short of them takes more, from what is left, in that feature's own random order and at most three of a family.
    for feature in FEATURES if floors else ():
        have = [r for r in chosen.values() if feature in r['features']]
        short = FEATURE_FLOOR - len(have)
        if short <= 0:
            continue
        count = collections.Counter(r['family'] for r in have)
        for r in in_order([x for x in pop if feature in x['features'] and x['id'] not in chosen], 'floor|' + feature):
            if short <= 0:
                break
            if count[r['family']] >= 3:
                continue
            chosen[r['id']] = r
            count[r['family']] += 1
            short -= 1
        if short > 0:
            for r in in_order([x for x in pop if feature in x['features'] and x['id'] not in chosen], 'floor2|' + feature)[:short]:
                chosen[r['id']] = r
    return chosen, set(chosen) - drawn


def pick_quick(standard, rate, landmark_rows):
    """A part of standard: the small landmarks, then cells by weight, then a document for any feature with none.  One long document, so that books are not missing."""
    fast = {i: r for i, r in standard.items() if pass_seconds(r, rate) <= QUICK_MAX_S}
    chosen = {r['id']: r for r in landmark_rows if r['id'] in fast and r['id'].startswith('ladder/') and r['size'] == 'one-page'}
    cells = collections.defaultdict(list)
    for r in fast.values():
        cells[cell_of(r)].append(r)
    for round_no in range(10):
        for c in sorted(cells, key=lambda c: (-len(cells[c]), c)):
            if len(chosen) >= QUICK_N:
                break
            for r in in_order(cells[c], f'quick|{c}|{round_no}'):
                if r['id'] not in chosen:
                    chosen[r['id']] = r
                    break
    for feature in FEATURES:
        if not any(feature in r['features'] for r in chosen.values()):
            for r in in_order([x for x in fast.values() if feature in x['features']], 'quick|' + feature):
                chosen[r['id']] = r
                break
    book = sorted((r for r in standard.values() if r['size'] == 'book' and r['id'] not in chosen), key=lambda r: pass_seconds(r, rate))
    if book:
        chosen[book[len(book) // 3]['id']] = book[len(book) // 3]
    return chosen


def pick_quota(standard, quota, salt, rate, limit, need_edits=False):
    cells = collections.defaultdict(list)
    for r in standard.values():
        if pass_seconds(r, rate) > limit:
            continue
        if need_edits and not r.get('private') and not {'word-start', 'word-middle', 'word-end', 'sentence'} <= set(r.get('edit_kinds') or ()):
            continue
        cells[r['size']].append(r)
    res = {}
    for size, n in quota.items():
        for r in draw(cells[size], n, salt + '|' + size, cap_share=0.25):
            res[r['id']] = r
    return res


def weights(pop, tier, mass, landmarks=(), floors=()):
    """A document's weight in a pooled figure, so that the figure estimates the population by mass, whatever went into the tier for other reasons.  Within each cell, whose total stays its mass:

    - A landmark was taken by name, so it stands for itself: its own mass.
    - What a floor added was taken for having a feature, so it stands for nothing else: no weight, and it stays in its feature's stratum.  A pooled figure with such rows would hold that feature several times its share.
    - A family past FAMILY_MASS counts as that many in every share, and the draw takes documents and not families, so it is a stratum of its own: its mass in the cell, shared by what was drawn of it.
    - The rest of the cell's mass is shared by what was drawn of the rest.

    Mass with nothing drawn to carry it goes to what was drawn in the cell, or if nothing was, to the cell's landmarks and floors, in equal parts, so that a cell keeps its share."""
    res = {}
    cells = collections.defaultdict(list)
    for r in pop:
        cells[cell_of(r)].append(r)
    for c, rows in cells.items():
        here = [r for r in rows if r['id'] in tier]
        if not here:
            continue
        w = {r['id']: mass[r['family']] for r in here if r['id'] in landmarks}
        group = lambda r: r['family'] if mass[r['family']] < 1.0 else ''      # noqa: E731
        share, drawn = collections.Counter(), collections.defaultdict(list)
        for r in rows:
            if r['id'] not in landmarks:
                share[group(r)] += mass[r['family']]
        for r in here:
            if r['id'] not in landmarks and r['id'] not in floors:
                drawn[group(r)].append(r['id'])
        spill = 0.0
        for g, m in share.items():
            if drawn[g]:
                for i in drawn[g]:
                    w[i] = m / len(drawn[g])
            else:
                spill += m
        if spill:
            carriers = [i for g in sorted(drawn) for i in drawn[g]] or sorted(r['id'] for r in here)
            for i in carriers:
                w[i] = w.get(i, 0.0) + spill / len(carriers)
        for i, x in w.items():
            res[i] = round(x, 4)
    return res


# ------------------------------------------------------------------------------------------------ diagrams
def arrow_bucket(arrows):
    for most, name in ARROW_BUCKETS:
        if arrows <= most:
            return name
    return ARROW_BUCKETS[-1][1]


def pick_diagrams(rows, apart, training):
    """Each diagram that pdflatex compiles alone with real tikz-cd, from a document people wrote.  What commute can handle was not asked: what it declines stays in, and counts as a fallback.  Diagrams of a family kept apart go to the hold-out."""
    usable = [d for d in rows if d.get('standalone_pdf') and d.get('standalone_rc') == 0 and d['group'] in DIAGRAM_SOURCES]
    for d in usable:
        d['stratum'] = f'{d["group"]}/{arrow_bucket(d["arrows"])}/{"fancy" if d["fancy"] else "plain"}'
        d['key'] = d['entry'] + '#' + d['unit']
        d['apart'] = split_family(family_of_doc(d['source_doc']), d['source_doc']) in apart
        d['trained'] = d['source_doc'] in training
    side = {False: [d for d in usable if not d['apart']], True: [d for d in usable if d['apart']]}

    def sample(pool, n, salt, floor=2):
        cells = collections.defaultdict(list)
        for d in pool:
            cells[d['stratum']].append(d)
        alloc = largest_remainder({c: (len(v), floor, len(v)) for c, v in cells.items()}, n)
        res = {}
        for c in sorted(cells):
            order = in_order(cells[c], salt + '|' + c, key=lambda d: d['key'])
            count = collections.Counter()
            taken = []
            for d in order:
                if len(taken) >= alloc[c]:
                    break
                if count[d['source_doc']] < 4:
                    taken.append(d)
                    count[d['source_doc']] += 1
            have = {d['key'] for d in taken}
            taken += [d for d in order if d['key'] not in have][:alloc[c] - len(taken)]
            for d in taken:
                res[d['key']] = d
        return res
    standard = sample(side[False], DIAGRAMS_STANDARD, 'diagrams-standard')
    quick = sample(list(standard.values()), DIAGRAMS_QUICK, 'diagrams-quick', floor=0)
    holdout = sample(side[True], DIAGRAMS_HOLDOUT, 'diagrams-holdout')
    return usable, standard, quick, holdout


def family_of_doc(doc_id):
    """As census.py's group_of, for a document named by a diagram."""
    p = doc_id.split('/')
    if p[0] == 'arxiv':
        return doc_id
    if p[0] in ('stacks', 'stacks-xr'):
        return 'stacks'
    if p[0] == 'docs':
        return '/'.join(p[:3])
    return '/'.join(p[:2])


# ------------------------------------------------------------------------------------------------ what it costs
def pc_rate(rows):
    """Oracle instructions per millisecond of CPU on the PC, the median over documents of at least a second, for what its cache does not cover.  The cache was made at some -j, so a time from it is on the long side."""
    vals = [r['instr_total'] / r['cpu_passes_pc'][-1] for r in rows if r.get('instr_total') and r.get('cpu_passes_pc') and r['cpu_passes_pc'][-1] >= 1000]
    return statistics.median(vals) if vals else 4e6


def pass_seconds(r, rate):
    """One last pass of the oracle on the PC: from its own cache, or for a document it has not, from the instructions here."""
    cpu = r.get('cpu_passes_pc')
    if cpu:
        return cpu[-1] / 1000.0
    return (r.get('instr_total') or 0) / rate / 1000.0


def first_seconds(r, rate):
    """From nothing: the passes the oracle took on the PC, or the passes here in the proportion of their CPU."""
    cpu = r.get('cpu_passes_pc')
    if cpu:
        return sum(cpu) / 1000.0
    cpu = r.get('pass_cpu_ms') or []
    scale = sum(cpu) / cpu[-1] if len(cpu) > 0 and cpu[-1] else float(r.get('passes') or 1)
    return pass_seconds(r, rate) * scale


# What runs where, for `estimate`.  A scenario has a `unit`, the seconds of one run of a document, and its sides as (runs of a pdfTeX, runs of rstex): `counted`, one run per side at -j COUNTED_JOBS on 16 cores, as
# instructions do not care; `timed`, serial on one pinned core in a round with a control.  A run of rstex is taken as RSTEX_BOUND of a run of pdflatex: the rule may not know better, so that gives an upper bound, and a second figure says what half of that would come to.
COUNTED_JOBS = 14
LATEXMK_S = 0.12
INF = 10.0 ** 9
RSTEX_BOUND, RSTEX_WHAT_IF = 1.0, 0.5
SCENARIOS = {
    'first': {'unit': 'first', 'counted': (2, 1), 'timed': (2, 1)},        # latexmk over each pdfTeX, rstex --until-stable
    'last-cold': {'unit': 'pass', 'counted': (2, 1), 'timed': (2, 2)},     # each pdfTeX, rstex; timed with rstex again, the A/A control
    'warm': {'unit': 'pass', 'counted': (0, 3), 'timed': (0, 2)},          # a write, a seal, the restore; timed, the restore and its control
    'warm-word': {'unit': 'pass', 'counted': (2, 3), 'timed': (0, 0)},     # the same in a mirror, then a word in the body, on each side
    'hits': {'unit': 'pass', 'counted': (0, 1), 'timed': (0, 2)},          # the same with the old PDF in place
    'resume': {'unit': 'pass', 'counted': (0, 2), 'timed': (0, 2)},        # a chain written, then from its last record
    'old-map': {'unit': 'pass', 'counted': (2, 0), 'timed': (0, 0)},       # each pdfTeX under the old map
    'edit': {'unit': 'edit', 'counted': (1, 1), 'timed': (0, 0)},          # seven saves, each on rstex and on latexmk over Arch's: a session of its own
    'native': {'unit': 'pass', 'counted': (0, 2), 'timed': (0, 2)},        # tikz-cd documents with and without the native path
}
# Which scenarios a tier has, up to how many seconds of the oracle's last pass a document may take in each, and how many timed rounds a document of that many seconds gets.
PLAN = {
    'quick': {'counted': {'first': INF, 'last-cold': INF, 'warm': INF, 'warm-word': INF, 'hits': INF, 'resume': INF, 'edit': INF, 'native': INF},
              'timed': {'last-cold': 10.0, 'warm': 10.0}, 'rounds': ((0.5, 3), (INF, 1))},
    'standard': {'counted': {'first': 30.0, 'last-cold': INF, 'warm': INF, 'warm-word': 30.0, 'hits': 30.0, 'resume': 30.0, 'edit': INF, 'native': 30.0, 'old-map': 10.0},
                 'timed': {'last-cold': 10.0, 'warm': 10.0, 'hits': 2.0, 'native': 10.0}, 'rounds': ((0.5, 3), (INF, 1))},
    'holdout': {'counted': {'first': 30.0, 'last-cold': INF, 'warm': INF, 'warm-word': 30.0, 'hits': 30.0, 'resume': 30.0, 'edit': INF, 'native': 30.0},
                'timed': {'last-cold': 10.0, 'warm': 10.0}, 'rounds': ((INF, 1),)},
    'full': {'counted': {'first': INF, 'last-cold': INF, 'warm': INF, 'warm-word': 20.0, 'hits': INF, 'resume': INF, 'edit': 20.0, 'native': INF, 'old-map': INF},
             'timed': {'last-cold': INF, 'warm': INF}, 'rounds': ((INF, 1),)},
}
CHECK_S, CHECK_S_PER_PAGE = 0.1, 0.07   # the identical-output check of a row, tests/harness/compare_pdf.py: 82 s on the 1,180 pages of the PGF manual
FMT_RUNS = {'quick': 1, 'standard': 3, 'holdout': 1, 'full': 3}
PIXEL_ROUNDS = 4


def scenario_cost(name, r, rate):
    """Seconds of a run on (a pdfTeX side, an rstex side)."""
    t, first = pass_seconds(r, rate), first_seconds(r, rate)
    unit = SCENARIOS[name]['unit']
    if unit == 'first':
        return first + LATEXMK_S, first
    if unit == 'edit':
        return 14 * t + first, 7 * t + first     # a build to the fixed point on each side, then seven saves: two passes under latexmk, one direct
    return t, t


def applies(name, r, flags, tier):
    # A regression test is run as its suite runs it, by one pass or by its own: no latexmk, no edits.
    if r['kind'] == 'testfile' and name not in ('last-cold', 'warm'):
        return False
    if name == 'edit':
        return tier == 'full' or flags.get('edit', False)
    if name == 'native':
        return r.get('tikzcd', False)
    return True


def rounds_for(tier, t):
    for most, n in PLAN[tier]['rounds']:
        if t <= most:
            return n
    return 0


def estimate(tier, rows, flags, rate, diagrams=0, diagram_s=0.0):
    """Seconds on the PC: an upper bound, and what it would be with rstex at half the bound's cost."""
    res = {}
    for label, f in (('bound', RSTEX_BOUND), ('half', RSTEX_WHAT_IF)):
        counted = timed = 0.0
        by = collections.Counter()
        plan = PLAN[tier]
        for r in rows:
            t = pass_seconds(r, rate)
            check = CHECK_S + CHECK_S_PER_PAGE * (r.get('pages') or 0)
            for name, most in plan['counted'].items():
                if t > most or not applies(name, r, flags.get(r['id'], {}), tier):
                    continue
                s = SCENARIOS[name]
                on_tex, on_rstex = scenario_cost(name, r, rate)
                c = s['counted'][0] * on_tex + s['counted'][1] * on_rstex * f + check
                counted += c
                by[name + ' counted'] += c
                limit = plan['timed'].get(name)
                # In full a regression test is only counted: 1,000 of them say little more than the 16 of standard.
                if limit is not None and t <= limit and not (tier == 'full' and r['kind'] == 'testfile'):
                    x = rounds_for(tier, t) * (s['timed'][0] * on_tex + s['timed'][1] * on_rstex * f)
                    timed += x
                    by[name + ' timed'] += x
        fmt = 72e9 / rate / 1000.0 * (2 + f) * FMT_RUNS[tier]
        by['pdflatex.fmt, three engines, timed'] = fmt
        timed += fmt
        if tier == 'standard':
            pixels = sum(PIXEL_ROUNDS * 3 * pass_seconds(r, rate) * f + 0.2 for r in rows if flags.get(r['id'], {}).get('pixels'))
            by['save to pixels, timed'] = pixels
            timed += pixels
        if diagrams:
            # Each pdfTeX with real tikz-cd, and commute, which is next to nothing.  full only counts them.
            d = diagrams * (2 * diagram_s + 0.05)
            if tier == 'full':
                by['diagrams, counted'] = d
                counted += d
            else:
                by['diagrams, timed'] = d
                timed += d
        res[label] = {'counted_s': round(counted), 'timed_s': round(timed), 'wall_s': round(counted / COUNTED_JOBS + timed), 'by': {k: round(v) for k, v in sorted(by.items())}}
    return {'counted_jobs': COUNTED_JOBS, **res}


def plan_for_file():
    return {t: {'counted': {k: (None if v == INF else v) for k, v in p['counted'].items()}, 'timed': {k: (None if v == INF else v) for k, v in p['timed'].items()},
                'rounds': [[None if a == INF else a, b] for a, b in p['rounds']]} for t, p in PLAN.items()}


# ------------------------------------------------------------------------------------------------ the whole rule
def entry_row(r, tiers, w, landmark, trained, family_trained, flags, floor=False):
    e = {'id': r['id'], 'tiers': tiers, 'kind': r['kind'], 'size': r['size'], 'family': r['family'], 'field': r['field'], 'pages': r.get('pages'), 'passes': r.get('passes'),
         'minstr': round(r['instr_total'] / 1e6, 1), 'source_closure': r['source_closure'], 'tree_closure': r['tree_closure'], 'on_pc': bool(r.get('on_pc'))}
    if r.get('features'):
        e['features'] = r['features']
    if r.get('preamble_share') is not None:
        e['preamble_share'] = round(r['preamble_share'], 3)
    if w:
        e['weight'] = w
    if floor:
        e['entered'] = 'floor'
    if landmark:
        e['landmark'] = landmark
    if trained:
        e['pgo'] = 'trained'
    elif family_trained:
        e['pgo'] = 'family'
    if r.get('errors'):
        e['errors'] = r['errors']
    if r.get('population') == 'micro':
        e['synthetic'] = True
    if r.get('year'):
        e['year'] = r['year']
    for k in ('edit', 'pixels'):
        if flags.get(k):
            e[k] = True
    return e


def select_population(rows, training, landmarks, target_n, holdout_n, testfiles, rate, private=False):
    """The steps above for one population.  `rows` are its eligible documents, derived."""
    mass = family_mass(rows)
    by_id = {r['id']: r for r in rows}
    tests = [r for r in rows if r['kind'] == 'testfile']
    real = [r for r in rows if r['kind'] != 'testfile']
    landmark_rows = [by_id[i] for i in sorted(landmarks) if i in by_id]
    missing = sorted(i for i in landmarks if i not in by_id)
    named = set(TOOLS_NAMED) | training | set(landmarks)
    touched = {by_id[i]['family'] for i in named if i in by_id}
    # Of the owner's documents, those the PC holds, and none that would take as long as the rest together: one takes 400 s a pass.
    real_on_pc = [r for r in real if (r.get('on_pc') and pass_seconds(r, rate) <= PRIVATE_MAX_S) or r['id'] in landmarks] if private else real
    apart = choose_holdout([r for r in real_on_pc if r['id'] not in landmarks], touched, mass, [r for r in real_on_pc if r['id'] in landmarks])
    test_apart = choose_holdout(tests, touched, mass) if tests else set()
    apart |= test_apart
    not_apart = [r for r in real_on_pc if r['family'] not in apart]
    standard, floors = pick_standard(not_apart, mass, landmark_rows, target_n, floors=not private)
    # The hold-out: the same cells, drawn in proportion from the families kept apart, at the size its share gives.
    pool = [r for r in real_on_pc if r['family'] in apart]
    holdout = {}
    if pool:
        alloc, cells = allocate(pool, family_mass(pool), min(holdout_n, len(pool)), floor=1)
        for c in sorted(cells):
            for r in draw(cells[c], alloc[c], 'holdout|' + c, cap_share=0.34):
                holdout[r['id']] = r
        drawn_holdout = set(holdout)
        # A feature the proportions missed takes a few, from what the families kept apart have of it.
        for feature in FEATURES if not private else ():
            short = HOLDOUT_FEATURE_FLOOR - sum(1 for r in holdout.values() if feature in r['features'])
            for r in in_order([x for x in pool if feature in x['features'] and x['id'] not in holdout], 'holdout-floor|' + feature)[:max(0, short)]:
                holdout[r['id']] = r
        floors |= set(holdout) - drawn_holdout
    # Regression tests, by suite: a small stratum of their own.
    test_pool = [r for r in tests if r['family'] not in apart]
    test_alloc, test_cells = ({}, {}) if not test_pool else allocate_families(test_pool, testfiles[0])
    for f in sorted(test_cells):
        for r in draw(test_cells[f], test_alloc[f], 'testfile|' + f):
            standard[r['id']] = r
    for r in in_order([x for x in tests if x['family'] in apart], 'testfile-holdout')[:testfiles[2]]:
        holdout[r['id']] = r
    if private:
        quick = {r['id']: r for r in landmark_rows if pass_seconds(r, rate) <= QUICK_MAX_S}
    else:
        quick = pick_quick({i: r for i, r in standard.items() if r['kind'] != 'testfile'}, rate, landmark_rows)
    for r in in_order([x for x in standard.values() if x['kind'] == 'testfile'], 'testfile-quick')[:testfiles[1]]:
        quick[r['id']] = r
    full = [r for r in rows if r['family'] not in apart and (not private or (r.get('on_pc') and pass_seconds(r, rate) <= PRIVATE_MAX_S) or r['id'] in landmarks)]
    return {'eligible': rows, 'mass': mass, 'apart': apart, 'touched': touched, 'landmark_rows': landmark_rows, 'missing': missing, 'standard': standard, 'quick': quick, 'holdout': holdout, 'full': full, 'not_apart': not_apart, 'tests': tests, 'pool': pool, 'floors': floors}


def allocate_families(pool, n):
    """Regression tests have no cells, only suites: share in proportion to the root of a suite's size, so that the largest does not take all."""
    fams = collections.defaultdict(list)
    for r in pool:
        fams[r['family']].append(r)
    want = {f: (math.sqrt(len(v)), 0, len(v)) for f, v in fams.items()}
    return largest_remainder(want, n), fams


def build(rows_raw, diagram_rows, tree, private_landmarks):
    """Everything from the census.  Returns (public document, private document or None)."""
    for r in rows_raw:
        bad = forbidden_in(r)
        if bad:
            die(f'a census row has {bad}: this rule must not see anything rstex produced')
    rows, floor, text_limit = derive(rows_raw)
    all_ids = {r['id'] for r in rows}
    training = training_ids(all_ids)
    public = [r for r in rows if not r.get('private')]
    private = [r for r in rows if r.get('private')]
    rate = pc_rate(public)
    # A public document has to be normal on both machines: the PC is where reports are measured, and its tree has to give the oracle the same class.
    pop_public = [r for r in public if eligible(r) and r.get('on_pc') and (r['population'] in ('real', 'l3build') or r['population'] == 'micro' and r['id'].startswith('coverage/cls/') or r['id'] in LANDMARKS)]
    s = select_population(pop_public, {i for i in training if i in all_ids and not is_private_id(i)}, LANDMARKS, STANDARD_N, HOLDOUT_N, (TESTFILES_STANDARD, TESTFILES_QUICK, TESTFILES_HOLDOUT), rate)
    doc_public = assemble(s, rate, floor, text_limit, training, tree, diagram_rows, public_side=True, census=rows_raw)
    doc_private = None
    if private and private_landmarks is not None:
        pop_private = [r for r in private if eligible(r)]
        sp = select_population(pop_private, {i for i in training if is_private_id(i)}, private_landmarks, PRIVATE_STANDARD_N, PRIVATE_HOLDOUT_N, (0, 0, 0), rate, private=True)
        doc_private = assemble(sp, rate, floor, text_limit, training, tree, None, public_side=False, census=rows_raw)
    return doc_public, doc_private


def is_private_id(i):
    return i.startswith(('user/', 'local/', 'cd-local/'))


def census_summary(rows_raw):
    """How the public population splits, for the README and for a later check that the census is the same."""
    c = collections.Counter()
    for r in rows_raw:
        if not r.get('private'):
            c[f'{r.get("population")}/{r.get("class")}'] += 1
    return dict(sorted(c.items()))


def assemble(s, rate, floor, text_limit, training, tree, diagram_rows, public_side, census):
    mass = s['mass']
    standard, quick, holdout, full = s['standard'], s['quick'], s['holdout'], s['full']
    landmarks = {r['id'] for r in s['landmark_rows']}
    # Scenario subsets of standard.
    edit = pick_quota({i: r for i, r in standard.items() if r['kind'] != 'testfile'}, EDIT_QUOTA, 'edit', rate, EDIT_MAX_S, need_edits=True)
    for r in s['landmark_rows']:
        if pass_seconds(r, rate) <= EDIT_MAX_S and r['size'] in ('book', 'long-paper', 'paper') and {'word-start', 'word-middle', 'word-end', 'sentence'} <= set(r.get('edit_kinds') or ('word-start', 'word-middle', 'word-end', 'sentence')):
            edit[r['id']] = r
    pixels = pick_quota(edit, PIXEL_QUOTA, 'pixels', rate, EDIT_MAX_S)
    for r in quick.values():
        if r['size'] in ('note', 'paper') and {'word-start', 'word-middle', 'word-end', 'sentence'} <= set(r.get('edit_kinds') or ()) and len([x for x in quick if x in edit]) < 6:
            edit[r['id']] = r
    # The hold-out has an edit session on as many rows, in proportion.
    held = pick_quota({i: r for i, r in holdout.items() if r['kind'] != 'testfile'}, {k: max(1, v // 2) if v else 0 for k, v in EDIT_QUOTA.items()}, 'edit-holdout', rate, EDIT_MAX_S, need_edits=True)
    flags = collections.defaultdict(dict)
    for i in list(edit) + list(held):
        flags[i]['edit'] = True
    for i in pixels:
        flags[i]['pixels'] = True
    # Letter stand-ins and the rungs of the ladder are landmarks of their own, written for this project and not to be read, so they are no part of what a pooled figure estimates.
    pooled_row = lambda r: r['kind'] != 'testfile' and r.get('population') not in ('micro', 'ladder')       # noqa: E731
    real_not_apart = [r for r in s['not_apart'] if pooled_row(r)]
    w_standard = weights(real_not_apart, {i: r for i, r in standard.items() if pooled_row(r)}, mass, landmarks, s['floors'])
    # In the hold-out, a cell's share is that of the population the rule was drawn from.
    pop_mass = collections.defaultdict(float)
    for r in real_not_apart:
        pop_mass[cell_of(r)] += mass[r['family']]
    w_holdout = weights([r for r in s['pool'] if pooled_row(r)], {i: r for i, r in holdout.items() if pooled_row(r)}, mass, (), s['floors'])
    held_total = collections.defaultdict(float)
    for i, x in w_holdout.items():
        held_total[cell_of(holdout[i])] += x
    w_holdout = {i: round(x * pop_mass[cell_of(holdout[i])] / held_total[cell_of(holdout[i])], 4) for i, x in w_holdout.items() if pop_mass[cell_of(holdout[i])]}
    entries = []
    trained_family = {r['family'] for r in s['eligible'] if r['id'] in training}
    # The same contents under another id are the same document to a profile: `ladder/*` is trained on, and one rung is a file of the LaTeX documentation too.
    trained_closure = {r['source_closure'] for r in census if r['id'] in training and r.get('source_closure') and not (public_side and r.get('private'))}
    apart = s['apart']
    full_ids = {r['id'] for r in full}
    all_rows = {r['id']: r for r in s['eligible']}
    for r in sorted(all_rows.values(), key=lambda r: r['id']):
        i = r['id']
        if r['family'] in apart:
            tiers = ['holdout'] if i in holdout else ['reserve']
        else:
            tiers = [t for t, member in (('quick', i in quick), ('standard', i in standard), ('full', i in full_ids)) if member]
        if not tiers:
            continue
        w = w_holdout.get(i) if 'holdout' in tiers else w_standard.get(i)
        reason = (LANDMARKS.get(i) or 'the owner\'s own document') if i in landmarks else None
        entries.append(entry_row(r, tiers, w, reason, i in training or r['source_closure'] in trained_closure, r['family'] in trained_family, flags.get(i, {}), floor=i in s['floors']))
    doc = {'schema': 1, 'seed': SEED, 'part': 'public' if public_side else 'private', 'rule': rule_constants(), 'plan': plan_for_file(), 'tree': tree, 'pc_rate_instr_per_ms': round(rate), 'startup_floor_minstr': round(floor / 1e6, 1),
           'mostly_text_minstr_per_page': round(text_limit / 1e6, 2) if text_limit else None}
    if public_side:
        doc['census'] = census_summary(census)
        # The stress cases are micro documents on purpose: a side table in `full` on how things scale, never in a stratum and never pooled.
        doc['side_stress'] = sorted(r['id'] for r in census if r['id'].startswith('coverage/stress/') and eligible(r))
    counts = {t: sum(1 for e in entries if t in e['tiers']) for t in ('quick', 'standard', 'full', 'holdout', 'reserve')}
    doc['counts'] = counts
    doc['families_kept_apart'] = sorted(apart)
    doc['landmarks_missing'] = s['missing']
    diag = {}
    if diagram_rows is not None:
        usable, d_standard, d_quick, d_holdout = pick_diagrams([dict(d) for d in diagram_rows], apart, training)
        def d_rows(chosen):
            return [{'entry': d['entry'], 'unit': d['unit'], 'stratum': d['stratum'], 'arrows': d['arrows'], 'cells': d['cells'], 'minstr': round(d['standalone_instr'] / 1e6, 1), **({'pgo': 'trained'} if d['trained'] else {})} for _, d in sorted(chosen.items())]
        diag = {'usable': len(usable), 'all': len(diagram_rows), 'full_rule': 'every usable diagram of a family not kept apart', 'quick': d_rows(d_quick), 'standard': d_rows(d_standard), 'holdout': d_rows(d_holdout)}
        doc['diagrams'] = diag
    # Estimates.  Each tier's own documents, and the diagrams it has.
    diagram_s = statistics.median(d['minstr'] for d in diag['standard']) * 1e6 / rate / 1000 if diag.get('standard') else 0
    doc['estimate'] = {t: estimate(t, [all_rows[e['id']] for e in entries if t in e['tiers']], flags, rate, len(diag.get(t, ())) if t != 'full' else diag.get('usable', 0), diagram_s) for t in ('quick', 'standard', 'holdout', 'full')}
    doc['strata'] = strata_table(entries)
    doc['entries'] = entries
    return doc


def strata_table(entries):
    table = collections.defaultdict(collections.Counter)
    for e in entries:
        for t in e['tiers']:
            table[f'{e["kind"]}/{e["size"]}'][t] += 1
            for f in e.get('features', ()):
                table['feature:' + f][t] += 1
    return {k: dict(sorted(v.items())) for k, v in sorted(table.items())}


def rule_constants():
    return {'sizes': dict(SIZES), 'kinds': KINDS, 'features': FEATURES, 'family_mass': FAMILY_MASS, 'standard_n': STANDARD_N, 'cell_floor': CELL_FLOOR, 'family_cap': FAMILY_CAP, 'feature_floor': FEATURE_FLOOR, 'holdout_share': HOLDOUT_SHARE,
            'thresholds': {'tikz': T_TIKZ, 'tikzcd': T_TIKZCD, 'xy': T_XY, 'axis': T_AXIS, 'index': T_INDEX, 'images': T_IMAGES, 'table_rows': T_ROWS, 'long_rows': T_LONG_ROWS, 'preamble_share': T_PREAMBLE, 'text_quantile': T_TEXT_QUANTILE, 'text_pages': T_TEXT_PAGES}}


# ------------------------------------------------------------------------------------------------ writing
def dumps(doc):
    """One entry a line, so that a change to the set is a readable diff."""
    head = {k: v for k, v in doc.items() if k not in ('entries', 'diagrams', 'strata', 'estimate')}
    parts = ['{']
    for k, v in sorted(head.items()):
        parts.append(f' {json.dumps(k)}: {json.dumps(v, sort_keys=True)},')
    parts.append(f' "estimate": {json.dumps(doc.get("estimate"), sort_keys=True)},')
    parts.append(' "strata": {')
    strata = doc.get('strata', {})
    parts.append(',\n'.join(f'  {json.dumps(k)}: {json.dumps(v, sort_keys=True)}' for k, v in sorted(strata.items())))
    parts.append(' },')
    if 'diagrams' in doc:
        d = doc['diagrams']
        parts.append(' "diagrams": {')
        for k, v in sorted(d.items()):
            if not isinstance(v, list):
                parts.append(f'  {json.dumps(k)}: {json.dumps(v)},')
        lists = [k for k in d if isinstance(d[k], list)]
        for n, k in enumerate(sorted(lists)):
            parts.append(f'  {json.dumps(k)}: [')
            parts.append(',\n'.join('   ' + json.dumps(x, sort_keys=True) for x in d[k]))
            parts.append('  ]' + (',' if n + 1 < len(lists) else ''))
        parts.append(' },')
    parts.append(' "entries": [')
    parts.append(',\n'.join('  ' + json.dumps(e, sort_keys=True) for e in doc['entries']))
    parts.append(' ]')
    parts.append('}')
    return '\n'.join(parts) + '\n'


def write(path, doc):
    tmp = path + '.tmp'
    with open(tmp, 'w') as f:
        f.write(dumps(doc))
    os.replace(tmp, path)


def weighted_quantiles(pairs, fractions=(0.1, 0.25, 0.5, 0.75, 0.9)):
    pairs = sorted(pairs)
    total = sum(w for _, w in pairs)
    res, seen, k = [], 0.0, 0
    for f in fractions:
        while k < len(pairs) - 1 and seen + pairs[k][1] < f * total:
            seen += pairs[k][1]
            k += 1
        res.append(pairs[k][0])
    return res


def representativeness(doc, w):
    """Does a tier look like its population, on what the oracle says?  Weighted quantiles of each.  The population is `full` and what the families kept apart hold, each document at its family's mass."""
    real = [e for e in doc['entries'] if e['kind'] != 'testfile' and not e.get('synthetic')]
    sizes = collections.Counter(e['family'] for e in real)
    mass = {f: min(1.0, FAMILY_MASS / n) for f, n in sizes.items()}
    pop = [e for e in real if 'full' in e['tiers']]
    for key, label in (('minstr', 'Minstr, a last pass'), ('pages', 'pages'), ('preamble_share', 'preamble share')):
        w(f'{label}, p10 p25 median p75 p90:\n')
        base = [(e[key], mass[e['family']]) for e in pop if e.get(key) is not None]
        w('  full, by mass:      ' + '  '.join(f'{v:.4g}' for v in weighted_quantiles(base)) + '\n')
        for tier in ('standard', 'holdout', 'quick'):
            rows = [(e[key], e['weight']) for e in real if tier in e['tiers'] and e.get(key) is not None and e.get('weight')]
            if rows:
                w(f'  {tier + ", by weight:":<20}' + '  '.join(f'{v:.4g}' for v in weighted_quantiles(rows)) + '\n')


def show(doc, out=sys.stdout):
    w = out.write
    w(f'part {doc["part"]}, seed {doc["seed"]}: ' + ', '.join(f'{k} {v}' for k, v in doc['counts'].items()) + f', families kept apart {len(doc["families_kept_apart"])}\n')
    for t, e in doc['estimate'].items():
        b, h = e['bound'], e['half']
        w(f'{t}: {b["wall_s"] / 60:.0f} min on the PC at most, {h["wall_s"] / 60:.0f} min if rstex takes half of that: at most {b["counted_s"] / 3600:.2f} h counted over {e["counted_jobs"]} cores, {b["timed_s"] / 60:.0f} min timed\n')
        w('   ' + ', '.join(f'{k} {v}' for k, v in b['by'].items()) + '\n')
    representativeness(doc, w)
    for t in ('quick', 'standard', 'full', 'holdout'):
        rows = [e for e in doc['entries'] if t in e['tiers']]
        w(f'{t}: {sum(1 for e in rows if e.get("pgo") == "trained")} rows trained on, {sum(1 for e in rows if e.get("pgo") == "family")} of a family trained on, {sum(1 for e in rows if e.get("errors"))} with TeX errors, {sum(e.get("minstr", 0) for e in rows) / 1e3:.0f} Ginstr a pass\n')
    if doc['part'] == 'public':
        w('trained on: ' + ' '.join(e['id'] for e in doc['entries'] if e.get('pgo') == 'trained') + '\n')
        w('of a family trained on, by family: ' + ' '.join(f'{f} {n}' for f, n in sorted(collections.Counter(e['family'] for e in doc['entries'] if e.get('pgo') == 'family').items())) + '\n')
        for t in ('quick', 'standard', 'holdout'):
            d = doc.get('diagrams', {}).get(t)
            if d:
                w(f'diagrams, {t}: {len(d)}, by source: ' + ' '.join(f'{k} {v}' for k, v in sorted(collections.Counter(x['stratum'].split('/')[0] for x in d).items())) + ', by arrows: ' + ' '.join(f'{k} {v}' for k, v in sorted(collections.Counter(x['stratum'].split('/')[1] for x in d).items())) + '\n')
        if 'diagrams' in doc:
            w(f'diagrams: {doc["diagrams"]["usable"]} usable of {doc["diagrams"]["all"]}\n')
    for k, v in doc['strata'].items():
        w(f'  {k:<30}' + ' '.join(f'{t} {v.get(t, 0):<5}' for t in ('quick', 'standard', 'full', 'holdout', 'reserve')) + '\n')


def check_pins(census_path, out):
    """A later census against the pins: how many entries are on another input or another tree."""
    rows = {r['id']: r for r in read_jsonl(census_path)}
    status = 0
    for name in ('set.json', 'set.local.json'):
        path = os.path.join(out, name)
        if not os.path.exists(path):
            continue
        with open(path) as f:
            doc = json.load(f)
        source = [e['id'] for e in doc['entries'] if rows.get(e['id'], {}).get('source_closure') != e['source_closure']]
        tree = [e['id'] for e in doc['entries'] if rows.get(e['id'], {}).get('tree_closure') != e['tree_closure']]
        print(f'{name}: {len(doc["entries"])} entries, {len(source)} on another input, {len(tree)} on another tree')
        if doc['part'] == 'public':
            print('  ' + ' '.join(source[:20]))
        status |= bool(source or tree)
    return status


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--census', default=os.path.join(OUT, 'census.jsonl'))
    ap.add_argument('--diagrams', default=os.path.join(OUT, 'diagrams.jsonl'))
    ap.add_argument('--tree', default=os.path.join(OUT, 'tree.json'))
    ap.add_argument('--landmarks', default=os.path.join(HERE, 'landmarks.local.json'))
    ap.add_argument('--out', default=HERE)
    ap.add_argument('--show', action='store_true')
    ap.add_argument('--check', action='store_true')
    args = ap.parse_args()
    if args.check:
        sys.exit(check_pins(args.census, args.out))
    if args.show:
        with open(os.path.join(args.out, 'set.json')) as f:
            show(json.load(f))
        return
    rows = read_jsonl(args.census)
    diagrams = read_jsonl(args.diagrams) if os.path.exists(args.diagrams) else []
    with open(args.tree) as f:
        tree = json.load(f)
    landmarks = None
    if os.path.exists(args.landmarks):
        with open(args.landmarks) as f:
            landmarks = json.load(f)
    public, private = build(rows, diagrams, tree, landmarks)
    write(os.path.join(args.out, 'set.json'), public)
    show(public)
    if private is not None:
        write(os.path.join(args.out, 'set.local.json'), private)
        print(f'private part: {private["counts"]}; {len(private["landmarks_missing"])} landmarks missing; ' + ', '.join(f'{t} at most {e["bound"]["wall_s"] / 60:.0f} min' for t, e in private['estimate'].items()))


if __name__ == '__main__':
    main()
