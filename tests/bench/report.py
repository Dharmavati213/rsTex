#!/usr/bin/env -S python3 -I
"""report.py: from the JSON lines of tools/prof/benchset.py to the tables of the Statistics section of tests/bench/README.md.

    report.py FILE... [--md PATH] [--json PATH] [--tier T] [--resamples 10000] [--draft TEXT] [--estimate]

Each figure names its tier, scenario and baseline, and has `N / M identical` beside it.  Reports quote against the official pdflatex first, then against Arch's; a figure against Arch's alone is marked.  Everything is rstex over the baseline, so under 1 is
fewer.  Public documents are named by id, private ones by id alone and apart, and neither a text nor a file name exists in what this reads.

**What goes in.**  A row counts if the binary is the one its side's name means in the file and the entry's closures are the set's pins.  Any other is refused and counted: a run on another input is not on this set.  A row for clocks counts only if benchset.py marked it
`clock_ok`, and counted rows never give a clock.

**A row that differs or fails counts against rstex.**  It stays in M and is named.  As a wrong answer has no speed, it is charged: what the attempt took plus the baseline, over the baseline, which is what a user who falls back pays.  The figures are given charged, and an uncharged figure stands
beside them, not in their place.

**Strata.**  A geometric mean over documents by kind, size and feature, with a percentile bootstrap interval over documents from a fixed seed: a stratum under 10 documents gives its rows and no interval.  The pooled figure is weighted by each row's `weight`, or in `full` by family mass, with an
interval from resampling families within cells, as documents of a family are not independent, and it is given with and without the rows `pgo` marks.  Regression tests and letter stand-ins are never pooled with anything.  The hold-out and the private part are each shown apart.

The tables are built so a later page can read the same numbers from --json.
"""
import os
import sys

# select.py in this directory is not the standard library's select.
sys.path[:] = [p for p in sys.path if os.path.abspath(p or os.curdir) != os.path.dirname(os.path.abspath(__file__))]

import argparse  # noqa: E402
import collections  # noqa: E402
import hashlib  # noqa: E402
import json  # noqa: E402
import math  # noqa: E402
import random  # noqa: E402
import statistics  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))
RIGHT = ('identical', 'same-picture')
ANSWERS_NOTHING = ('not-applicable', 'taken-back', 'unavailable', 'not-comparable', 'no-reference')
PRIVATE_PREFIXES = ('user/', 'local/', 'cd-local/')
INTERVAL_MIN = 10
BASELINE_ORDER = ('official', 'arch')
# What each figure is: the name in a table, the engine's scenario and variant, the baseline's scenario, and which rows of a tier it can have.
FIGURES = (
    ('first', 'first', None, 'first', 'From nothing to the right PDF: rstex --until-stable, running what it names, against latexmk over the baseline'),
    ('last-cold', 'last-cold', None, 'last-cold', 'One pass, side files in place, no checkpoint'),
    ('warm', 'warm', 'restore', 'last-cold', 'A pass on the source the checkpoint was made from, restored where it can be, with the policy forced: the best a restore can do, and not a save, against one pass of the baseline'),
    ('warm-word', 'warm-word', None, 'warm-word', 'A pass with a word in the body, restored where it can be, against one pass of the baseline on the same source'),
    ('warm-write', 'warm', 'write', 'last-cold', 'The pass that writes the checkpoint'),
    ('hits', 'hits', None, 'last-cold', 'The same with the last PDF in place: the output cache'),
    ('resume', 'resume', 'last', 'last-cold', 'From the last page record, where one is taken'),
    ('resume-write', 'resume', 'write', 'last-cold', 'The pass that writes the chain of records'),
    ('edit:settle', 'edit', 'settle', 'edit', 'A first build with Lemma\'s flags'),
    ('edit:word-start', 'edit', 'word-start', 'edit', 'A word near the start'),
    ('edit:word-middle', 'edit', 'word-middle', 'edit', 'A word in the middle'),
    ('edit:word-end', 'edit', 'word-end', 'edit', 'A word near the end'),
    ('edit:sentence', 'edit', 'sentence', 'edit', 'A sentence that adds lines'),
    ('edit:cite', 'edit', 'cite', 'edit', 'A new citation'),
    ('edit:preamble', 'edit', 'preamble', 'edit', 'A changed preamble'),
    ('edit:noop', 'edit', 'noop', 'edit', 'A save that changes nothing'),
    ('native', 'native', 'native', 'last-cold', 'A document with diagrams, by the native path'),
    ('fmt', 'fmt', None, 'fmt', 'pdflatex.fmt built'),
    ('old-map', 'last-cold', None, 'old-map', 'One pass against the baseline under the old font map'),
    ('diagram-alone', 'diagram-alone', None, 'diagram-alone', 'A figure: commute against a standalone document with real tikz-cd'),
)
EDIT_FIRST = 'edit:settle'


def die(message):
    sys.stderr.write(f'report.py: {message}\n')
    sys.exit(2)


def is_private(doc_id):
    return doc_id.startswith(PRIVATE_PREFIXES)


# ------------------------------------------------------------------------------------------------ small statistics
def geomean(values):
    return math.exp(sum(math.log(v) for v in values) / len(values)) if values else None


def nearest(sorted_values, q):
    """Nearest rank, as proflib.percentile."""
    if not sorted_values:
        return None
    return sorted_values[max(0, min(len(sorted_values) - 1, math.ceil(q * len(sorted_values)) - 1))]


def rng_for(*labels):
    return random.Random(int(hashlib.sha256('|'.join(str(x) for x in ('rstex-bench-report-1',) + labels).encode()).hexdigest()[:16], 16))


def bootstrap(values, resamples, *labels):
    """Percentile interval, 95%, of a geometric mean, resampling the documents, from a seed that depends on nothing but what it is an interval of."""
    if len(values) < INTERVAL_MIN or resamples <= 0:
        return None
    logs = [math.log(v) for v in values]
    rng = rng_for(*labels)
    n = len(logs)
    means = sorted(sum(rng.choices(logs, k=n)) / n for _ in range(resamples))
    return math.exp(nearest(means, 0.025)), math.exp(nearest(means, 0.975))


def pooled_mean(pairs):
    total = sum(w for w, _ in pairs)
    return math.exp(sum(w * math.log(r) for w, r in pairs) / total) if total else None


def pooled_interval(rows, resamples, *labels):
    """rows: (cell, family, weight, ratio).  Resampling families within cells, as documents of a family are not independent.  A cell's weight stays its own in each resample."""
    if len(rows) < INTERVAL_MIN or resamples <= 0:
        return None
    cells = collections.defaultdict(lambda: collections.defaultdict(list))
    for cell, family, w, r in rows:
        cells[cell][family].append((w, math.log(r)))
    rng = rng_for(*labels)
    prepared = []
    for cell, fams in sorted(cells.items()):
        groups = [fams[f] for f in sorted(fams)]
        weight = sum(w for g in groups for w, _ in g)
        prepared.append((weight, [(sum(w * x for w, x in g), sum(w for w, _ in g)) for g in groups]))
    total = sum(w for w, _ in prepared)
    means = []
    for _ in range(resamples):
        acc = 0.0
        for weight, groups in prepared:
            picks = rng.choices(groups, k=len(groups))
            sw = sum(p[1] for p in picks)
            acc += weight * (sum(p[0] for p in picks) / sw if sw else 0.0)
        means.append(acc / total)
    means.sort()
    return math.exp(nearest(means, 0.025)), math.exp(nearest(means, 0.975))


def spread(values):
    v = sorted(values)
    if not v:
        return None
    return {'min': v[0], 'p25': nearest(v, 0.25), 'median': nearest(v, 0.5), 'p75': nearest(v, 0.75), 'p90': nearest(v, 0.9), 'max': v[-1]}


# ------------------------------------------------------------------------------------------------ what goes in
def load_rows(paths):
    heads, rows, ends, jobs, errors = [], [], [], [], []
    for path in paths:
        with open(path) as f:
            for line in f:
                line = line.strip()
                if not line:
                    continue
                try:
                    r = json.loads(line)
                except ValueError:      # a line a kill cut short, in a file no run has gone on with
                    continue
                kind = r.get('kind')
                {'run': heads, 'm': rows, 'end': ends, 'job': jobs, 'harness-error': errors}.get(kind, []).append(r)
    return heads, rows, ends, jobs, errors


def load_set(set_path, local_path):
    with open(set_path) as f:
        public = json.load(f)
    entries = {e['id']: e for e in public['entries']}
    private = {}
    if local_path and os.path.exists(local_path):
        with open(local_path) as f:
            private = {e['id']: e for e in json.load(f)['entries']}
    return public, entries, private


def expected_binaries(heads):
    """What each side's name means in this file.  Two binaries under one name are not one side, and nothing here can say which was meant."""
    res = {}
    for h in heads:
        sides = {k: v['sha'] for k, v in h['sides'].items()}
        if h.get('commute'):
            sides['commute'] = h['commute']
        for label, sha in sides.items():
            if res.setdefault(label, sha) != sha:
                die(f'the side {label} has two binaries in these files, {res[label]} and {sha}.  Refused: name them apart')
    return res


def admit(rows, heads, entries):
    """(rows that count, {reason: count}, {reason: public ids}).  A duplicate keeps its first row: benchset.py makes none."""
    expected = expected_binaries(heads)
    trees = {h['tree'] for h in heads}
    tree = next(iter(trees)) if len(trees) == 1 else None
    good, refused, named, seen = [], collections.Counter(), collections.defaultdict(list), set()
    for r in rows:
        why = None
        e = entries.get(r['id'])
        side = r.get('side')
        if r['key'] in seen:
            why = 'a duplicate of a row before it'
        elif side in expected and r.get('binary_sha') and r['binary_sha'] != expected[side]:
            why = 'from another binary'
        elif side not in expected and side != '*':
            why = 'a side no header names'
        elif e is not None and 'source_closure' in r and r.get('check') != 'unavailable' and (r['source_closure'] != e.get('source_closure') or r.get('tree_closure') != e.get('tree_closure')):
            why = 'on another input or tree of the document'
        elif tree is not None and r.get('tree') != tree:
            why = 'on another TeX tree'
        if why:
            refused[why] += 1
            if not is_private(r['id']):
                named[why].append(r['id'])
            continue
        seen.add(r['key'])
        good.append(r)
    return good, refused, named


# ------------------------------------------------------------------------------------------------ pairs
class Tables:
    """The rows by what they are, for a tier and a part of the set."""

    def __init__(self, rows):
        self.counted = {}
        self.timed = collections.defaultdict(list)
        self.unavailable = collections.Counter()
        for r in rows:
            k = (r['id'], r['scenario'], r.get('variant'), r['side'])
            if r.get('check') == 'unavailable':
                self.unavailable[r['scenario']] += 1
                continue
            if r['method'] == 'counted':
                self.counted[k] = r
            else:
                self.timed[k].append(r)

    def row(self, doc, scenario, variant, side):
        """A counted row, or for what has only timed rows, one of those: instructions are the same."""
        r = self.counted.get((doc, scenario, variant, side))
        if r is not None:
            return r
        for v in (variant, 'a1', 'a2'):
            rows = self.timed.get((doc, scenario, v, side))
            if rows:
                return rows[0]
        return None


def pair_rows(t, ids, figure, engine, baseline):
    """[(id, ratio charged, ratio if it was right, row, baseline row)]; and how many have no row on one side or do not apply."""
    name, scenario, variant, base_scenario, _ = figure
    res = []
    absent = collections.Counter()
    for doc in ids:
        e = t.row(doc, scenario, variant, engine)
        if name == 'old-map' and e is None:
            continue
        b = t.row(doc, base_scenario, variant if base_scenario == 'edit' else None, baseline)
        if e is None or b is None:
            if e is not None or b is not None:
                absent['a row on one side only'] += 1
            continue
        if e.get('check') in ANSWERS_NOTHING or b.get('check') in ANSWERS_NOTHING:
            absent[e.get('check') if e.get('check') in ANSWERS_NOTHING else b.get('check')] += 1
            continue
        be, bb = e.get('instructions'), b.get('instructions')
        if name == 'old-map' and b.get('check') not in RIGHT:
            absent['renders otherwise under the old map, so no ratio'] += 1
            continue
        if not bb:
            absent['the baseline has no count'] += 1
            continue
        right = e.get('check') in RIGHT
        charged = ((be or 0) + bb) / bb if not right else be / bb
        res.append((doc, charged, be / bb if right else None, e, b))
    return res, absent


def stratum_of(entry):
    """Each document's strata, as the set names them."""
    out = ['kind:' + entry.get('kind', '?'), 'size:' + entry.get('size', '?')]
    out += ['feature:' + f for f in entry.get('features', ())]
    return out


def figure_table(pairs, entries, resamples, labels, weight_of):
    """The whole of a figure: the all-rows line, strata, pooled, the distribution and the worst ten."""
    real = [p for p in pairs if entries.get(p[0], {}).get('kind') != 'testfile' and not entries.get(p[0], {}).get('synthetic')]
    tests = [p for p in pairs if entries.get(p[0], {}).get('kind') == 'testfile']
    standins = [p for p in pairs if entries.get(p[0], {}).get('synthetic')]

    def line(part, *more):
        values = [p[1] for p in part]
        right = [p[2] for p in part if p[2] is not None]
        res = {'n': len(part), 'identical': len(right), 'charged': geomean(values), 'uncharged': geomean(right)}
        ci = bootstrap(values, resamples, *labels, *more)
        if ci:
            res['interval'] = ci
        return res
    res = {'real': line(real, 'real'), 'tests': line(tests, 'tests') if tests else None, 'standins': line(standins, 'standins') if standins else None, 'strata': {}}
    groups = collections.defaultdict(list)
    for p in real:
        for s in (stratum_of(entries[p[0]]) if p[0] in entries else ()):
            groups[s].append(p)
    for s, part in sorted(groups.items()):
        res['strata'][s] = line(part, s)
        if len(part) < INTERVAL_MIN:
            res['strata'][s]['rows'] = [{'id': p[0], 'ratio': p[1], 'right': p[2] is not None, 'pgo': entries.get(p[0], {}).get('pgo')} for p in part]
    res['distribution'] = spread([p[1] for p in real])
    res['worst'] = [{'id': p[0], 'ratio': round(p[1], 4), 'check': p[3].get('check'), 'pgo': entries.get(p[0], {}).get('pgo')} for p in sorted(real, key=lambda p: -p[1])[:10]]
    # Pooled.
    weighted = [(entries[p[0]].get('kind', '?') + '/' + entries[p[0]].get('size', '?'), entries[p[0]].get('family', p[0]), weight_of(p[0]), p[1], entries[p[0]].get('pgo')) for p in real if weight_of(p[0])]
    if weighted:
        all_rows = [(c, f, w, r) for c, f, w, r, _ in weighted]
        unmarked = [(c, f, w, r) for c, f, w, r, g in weighted if not g]
        res['pooled'] = {'n': len(all_rows), 'value': pooled_mean([(w, r) for _, _, w, r in all_rows]), 'interval': pooled_interval(all_rows, resamples, *labels, 'pooled')}
        if len(unmarked) < len(all_rows):
            res['pooled']['without_pgo_marked'] = {'n': len(unmarked), 'value': pooled_mean([(w, r) for _, _, w, r in unmarked]), 'interval': pooled_interval(unmarked, resamples, *labels, 'pooled-unmarked')}
    return res


def weights_for(tier, entries):
    """A row's weight in a pooled figure.  `full` has family mass, as select.py's representativeness has it."""
    if tier != 'full':
        return lambda i: entries.get(i, {}).get('weight')
    real = [e for e in entries.values() if e.get('kind') != 'testfile' and not e.get('synthetic') and 'full' in e.get('tiers', ())]
    sizes = collections.Counter(e.get('family') for e in real)
    mass = {f: min(1.0, 12 / n) for f, n in sizes.items()}
    return lambda i: mass.get(entries.get(i, {}).get('family'))


def clock_figures(t, ids, figure, engine, baseline):
    """Paired cycles with an A/A control, and CPU and wall, from rows marked clock_ok.  Within a round where the sides ran together, otherwise of medians."""
    name, scenario, variant, base_scenario, _ = figure
    if variant in ('write', 'native', 'settle') or scenario in ('edit', 'first', 'old-map'):
        return None
    if scenario in ('fmt', 'diagram-alone'):
        # Each side once a round, with no control in it: the medians, marked as such.
        cyc, cpu, wall = [], [], []
        for doc in ids:
            a = [r for r in t.timed.get((doc, scenario, None, engine), ()) if r.get('clock_ok')]
            b = [r for r in t.timed.get((doc, scenario, None, baseline), ()) if r.get('clock_ok')]
            if a and b:
                cyc.append(statistics.median(r['cycles'] for r in a) / statistics.median(r['cycles'] for r in b))
                cpu.append(statistics.median(r['cpu_ms'] for r in a) / statistics.median(r['cpu_ms'] for r in b))
                wall.append(statistics.median(r['wall_ms'] for r in a) / statistics.median(r['wall_ms'] for r in b))
        return {'n': len(cyc), 'control': None, 'cycles': geomean(cyc), 'cpu': geomean(cpu), 'wall': geomean(wall)} if cyc else None
    cyc, ctl, cpu, wall, n, identical = [], [], [], [], 0, 0
    for doc in ids:
        a1 = [r for r in t.timed.get((doc, scenario, 'a1', engine), ()) if r.get('clock_ok')]
        a2 = [r for r in t.timed.get((doc, scenario, 'a2', engine), ()) if r.get('clock_ok')]
        bs = [r for r in t.timed.get((doc, base_scenario, None, baseline), ()) if r.get('clock_ok')]
        if not a1 or not a2 or not bs:
            continue
        rounds = {r['round']: r for r in a1}
        rounds2 = {r['round']: r for r in a2}
        brounds = {r['round']: r for r in bs}
        per_round, per_control = [], []
        for k in sorted(set(rounds) & set(rounds2)):
            x, y = rounds[k].get('cycles'), rounds2[k].get('cycles')
            if not x or not y:
                continue
            per_control.append(x / y)
            b = brounds.get(k, bs[0]).get('cycles')
            if b:
                per_round.append((x + y) / 2 / b)
        if not per_round:
            continue
        n += 1
        # A row that is wrong has no speed: charged what it took plus the baseline, as in the instructions.  A timeout's wall is the limit.
        extra = 0 if all(r.get('check') in RIGHT for r in a1 + a2) else 1
        identical += 1 - extra
        cyc.append(statistics.median(per_round) + extra)
        ctl.append(statistics.median(per_control))
        cpu.append(statistics.median(r['cpu_ms'] for r in a1 + a2) / statistics.median(r['cpu_ms'] for r in bs) + extra)
        wall.append(statistics.median(r['wall_ms'] for r in a1 + a2) / statistics.median(r['wall_ms'] for r in bs) + extra)
    if not n:
        return None
    control = geomean(ctl)
    res = {'n': n, 'identical': identical, 'control': control, 'cpu': geomean(cpu), 'wall': geomean(wall)}
    if abs(control - 1) <= 0.01:
        res['cycles'] = geomean(cyc)
    else:
        res['cycles'] = None
        res['cycles_withheld'] = 'the control is more than 1% from 1'
    return res


# ------------------------------------------------------------------------------------------------ the report
def analyze(paths, set_path, local_path, resamples=10000, tier=None):
    heads, rows, ends, jobs, errors = load_rows(paths)
    if not heads:
        die('no run header in these files')
    public, entries, private = load_set(set_path, local_path)
    both = dict(entries)
    both.update(private)
    good, refused, named = admit(rows, heads, both)
    tiers = [tier] if tier else sorted({h['tier'] for h in heads})
    res = {'tiers': {}, 'refused': dict(refused), 'refused_public_ids': {k: sorted(set(v))[:30] for k, v in named.items()}, 'rows': len(rows), 'admitted': len(good), 'harness_errors': len(errors), 'runs': [run_summary(h) for h in heads], 'ends': ends}
    res['engines'] = sorted({label for h in heads for label, v in h['sides'].items() if v['kind'] == 'rstex'})
    res['baselines'] = [b for b in BASELINE_ORDER if any(b in h['sides'] for h in heads)]
    res['has_commute'] = any(h.get('commute') for h in heads)
    t_all = collections.defaultdict(list)
    for r in good:
        t_all[r['id']].append(r)
    for tier_name in tiers:
        res['tiers'][tier_name] = {}
        for part, pool in (('public', entries), ('private', private)):
            ids = [i for i, e in sorted(pool.items()) if tier_name in e.get('tiers', ())]
            ids = [i for i in ids if i in t_all]
            extra = []
            if part == 'public':
                # A format build, the diagrams and the stress cases have no row in the set: they belong to the tier that ran them.
                extra = sorted(i for i, rs in t_all.items() if i not in both and any(r.get('tier') == tier_name for r in rs))
            if not ids and not extra:
                continue
            res['tiers'][tier_name][part] = analyze_part(t_all, ids, extra, pool, tier_name, part, res, resamples)
    if 'standard' in res['tiers'] and 'holdout' in res['tiers']:
        res['holdout_outside'] = holdout_check(res['tiers'])
    if 'standard' in res['tiers'] and 'full' in res['tiers']:
        res['true_share'] = true_share(res['tiers'])
    return res


def run_summary(h):
    return {k: h.get(k) for k in ('tier', 'machine', 'cpus', 'governor', 'load', 'tree', 'tree_is_pin', 'floor', 'methods', 'jobs', 'cpu', 'max_load', 'thin', 'timed_engines', 'missing_tools', 'set_sha', 'resumed_rows', 'seed', 'commute')} | {'sides': {k: {'sha': v['sha'], 'kind': v['kind'], 'empty_cache_minstr': v.get('empty_cache_minstr')} for k, v in h['sides'].items()}}


def analyze_part(t_all, ids, extra, pool, tier, part, res, resamples):
    rows = [r for i in ids + extra for r in t_all[i]]
    t = Tables(rows)
    out = {'documents': len(ids), 'unavailable': dict(t.unavailable), 'figures': {}, 'counts': {}, 'wrong': [], 'memory': {}, 'marks': {}}
    weight_of = weights_for(tier, pool)
    out['marks'] = {'trained': [i for i in ids if pool[i].get('pgo') == 'trained' and not is_private(i)], 'family': [i for i in ids if pool[i].get('pgo') == 'family' and not is_private(i)], 'trained_n': sum(1 for i in ids if pool[i].get('pgo') == 'trained'), 'family_n': sum(1 for i in ids if pool[i].get('pgo') == 'family')}
    methods = collections.Counter()
    for r in rows:
        methods[(r['scenario'], r['method'], r['side'])] += 1
    out['counts'] = {f'{a}|{b}|{c}': v for (a, b, c), v in sorted(methods.items())}
    bases = res['baselines']
    for figure in FIGURES:
        name, scenario, variant, base, title = figure
        fig = {}
        engines = ['commute'] if scenario == 'diagram-alone' else res['engines']
        for engine in engines:
            for baseline in bases:
                if base == 'edit' and baseline != 'arch':
                    continue
                docs = ids + extra if scenario in ('fmt', 'diagram-alone') else ids
                pairs, absent = pair_rows(t, docs, figure, engine, baseline)
                if not pairs:
                    if absent:
                        fig.setdefault(engine, {})[baseline] = {'none': dict(absent)}
                    continue
                entries_view = pool if scenario not in ('fmt', 'diagram-alone') else {}
                if scenario == 'diagram-alone':
                    # A diagram's strata are its own: where it comes from, how many arrows, and whether it uses more than plain arrows.
                    entries_view = {p[0]: dict(zip(('kind', 'size', 'feature'), (p[3].get('stratum') or '//').split('/'))) for p in pairs}
                    for v in entries_view.values():
                        v['features'] = [v.pop('feature')] if v.get('feature') else []
                tab = figure_table(pairs, entries_view, resamples, (tier, part, name, engine, baseline), weight_of if scenario not in ('fmt', 'diagram-alone') else lambda i: None)
                tab['absent'] = dict(absent)
                c = clock_figures(t, docs, figure, engine, baseline)
                if c:
                    tab['clock'] = c
                memory = [(p[3].get('maxrss_kb'), p[4].get('maxrss_kb')) for p in pairs if p[3].get('maxrss_kb') and p[4].get('maxrss_kb')]
                faults = [(p[3].get('page_faults'), p[4].get('page_faults')) for p in pairs if p[3].get('page_faults') and p[4].get('page_faults')]
                size = [(p[3].get('pdf_bytes'), p[4].get('pdf_bytes')) for p in pairs if p[3].get('pdf_bytes') and p[4].get('pdf_bytes')]
                tab['also'] = {'peak_rss': geomean([a / b for a, b in memory]), 'page_faults': geomean([a / b for a, b in faults]), 'pdf_bytes': geomean([a / b for a, b in size]), 'peak_rss_kb': statistics.median(a for a, _ in memory) if memory else None, 'peak_rss_kb_baseline': statistics.median(b for _, b in memory) if memory else None}
                fig.setdefault(engine, {})[baseline] = tab
                for p in pairs:
                    if p[2] is None and baseline == bases[-1]:
                        out['wrong'].append({'id': p[0], 'figure': name, 'side': engine, 'check': p[3].get('check'), 'tex_errors': pool.get(p[0], {}).get('errors')})
        if fig:
            out['figures'][name] = {'title': title, 'by': fig}
    # The native path against real PGF on the same engine.
    native = {}
    for engine in res['engines']:
        n = [(i, t.row(i, 'native', 'native', engine), t.row(i, 'native', 'pgf', engine)) for i in ids]
        n = [x for x in n if x[1] and x[2] and x[1].get('instructions') and x[2].get('instructions')]
        if n:
            native[engine] = {'n': len(n), 'native_over_pgf': geomean([a['instructions'] / b['instructions'] for _, a, b in n]), 'identical_native': sum(1 for _, a, _ in n if a.get('check') in RIGHT), 'identical_pgf': sum(1 for _, _, b in n if b.get('check') in RIGHT)}
    out['native_over_pgf'] = native
    stress = [i for i in extra if i.startswith('coverage/stress/')]
    if stress:
        out['side_stress'] = {}
        for engine in res['engines']:
            for baseline in bases:
                pairs, _ = pair_rows(t, stress, FIGURES[1], engine, baseline)
                if pairs:
                    out['side_stress'][f'{engine}|{baseline}'] = [{'id': p[0], 'ratio': round(p[1], 4), 'check': p[3].get('check')} for p in pairs]
    # Whether the baselines' own answers were right, and the things only counted.
    baseline_wrong = collections.Counter()
    for r in rows:
        if r['side'] in BASELINE_ORDER and r.get('check') not in RIGHT + ('reference',) + ANSWERS_NOTHING:
            baseline_wrong[f'{r["side"]}/{r["scenario"]}'] += 1
    out['baseline_differs'] = dict(baseline_wrong)
    restored = collections.Counter()
    records = collections.Counter()
    for r in rows:
        if r.get('scenario') in ('warm', 'warm-word', 'hits', 'resume') and r.get('variant') in ('restore', None, 'last', 'a1', 'a2') and 'restored' in r:
            restored[(r['scenario'], bool(r['restored']))] += 1
        if r.get('scenario') == 'resume' and r.get('variant') == 'last':
            records['used a record' if r.get('used_record') else 'took none'] += 1
    out['restored'] = {f'{k[0]}|{"restored" if k[1] else "not restored"}': v for k, v in sorted(restored.items())}
    out['warm_not_restored'] = sorted({r['id'] for r in rows if r.get('scenario') == 'warm' and r.get('variant') == 'restore' and r.get('restored') is False})
    out['word_not_restored'] = sorted({r['id'] for r in rows if r.get('scenario') == 'warm-word' and r.get('restored') is False})
    out['resume_records'] = dict(records)
    edits_state = collections.Counter()
    syn = collections.Counter()
    for r in rows:
        if r.get('scenario') == 'edit' and r['side'] not in BASELINE_ORDER:
            state = 'not applicable' if r.get('applied') is False else 'taken back' if r.get('taken_back') else 'applied'
            edits_state[f'{r.get("variant")}|{state}'] += 1
            if r.get('synctex'):
                syn[r['synctex']] += 1
    out['edit_states'] = dict(sorted(edits_state.items()))
    against = collections.Counter(r.get('against_oracle') for r in rows if r.get('scenario') == 'first' and r.get('check') == 'reference')
    out['reference_against_oracle'] = {str(k): v for k, v in against.items()}
    out['synctex'] = dict(syn)
    # DESIGN.md: the native diagram path has no scaffolding boxes, so SyncTeX's text differs inside a diagram, and that is a named exception.  Whatever differs elsewhere is not.
    with_native = {r['id'] for r in rows if r['scenario'] == 'native'}
    out['synctex_differs_without_diagrams'] = sorted({r['id'] for r in rows if r.get('synctex') == 'differs' and r['id'] not in with_native})
    return out


def holdout_check(tiers):
    """A stratum of the hold-out outside the standard tier's interval is stated."""
    res = []
    std = tiers['standard'].get('public', {}).get('figures', {})
    held = tiers['holdout'].get('public', {}).get('figures', {})
    for name, fig in held.items():
        for engine, by in fig['by'].items():
            for baseline, tab in by.items():
                other = std.get(name, {}).get('by', {}).get(engine, {}).get(baseline)
                if not other or 'strata' not in tab or 'strata' not in other:
                    continue
                for s, line in tab['strata'].items():
                    o = other['strata'].get(s, {})
                    if o.get('interval') and line.get('charged') and not (o['interval'][0] <= line['charged'] <= o['interval'][1]):
                        res.append({'figure': name, 'engine': engine, 'baseline': baseline, 'stratum': s, 'holdout': line['charged'], 'standard_interval': o['interval']})
    return res


def true_share(tiers):
    """standard's weighted figure against full's on the same run, and a difference beyond the interval is stated, not repaired."""
    res = []
    std = tiers['standard'].get('public', {}).get('figures', {})
    full = tiers['full'].get('public', {}).get('figures', {})
    for name, fig in std.items():
        for engine, by in fig['by'].items():
            for baseline, tab in by.items():
                other = full.get(name, {}).get('by', {}).get(engine, {}).get(baseline)
                if tab.get('pooled', {}).get('interval') and other and other.get('pooled', {}).get('value'):
                    inside = tab['pooled']['interval'][0] <= other['pooled']['value'] <= tab['pooled']['interval'][1]
                    res.append({'figure': name, 'engine': engine, 'baseline': baseline, 'standard': tab['pooled']['value'], 'interval': tab['pooled']['interval'], 'full': other['pooled']['value'], 'inside': inside})
    return res


# ------------------------------------------------------------------------------------------------ Markdown
def num(x, digits=3):
    return f'{x:.{digits}f}' if isinstance(x, (int, float)) else ''


def ci(pair):
    return f'{pair[0]:.3f} to {pair[1]:.3f}' if pair else 'under 10 documents: its rows'


def markdown(res, draft=None):
    out = []
    w = out.append
    w('# Benchmark report')
    w('')
    if draft:
        w(f'**{draft}**')
        w('')
    for r in res['runs']:
        w(f'- Tier `{r["tier"]}`, seed `{r.get("seed")}`, on {r["machine"]}, {r["cpus"]} CPUs, governor {r["governor"]}, load {r["load"]} at the start, methods {", ".join(r["methods"])}, -j {r["jobs"]}, thinned: {r.get("thin") or "no"}.')
        w(f'- TeX tree `{r["tree"]}`, {"the one the set was made on" if r.get("tree_is_pin") else "NOT the one the set was made on"}. A peak under {r["floor"]["maxrss_kb"]} kB cannot be told from the runner\'s own floor. A run of /bin/true took {r["floor"].get("wall_ms", 0):.1f} ms of wall, which no time here has had taken off, so that a very short run\'s ratio is on the side against it.')
        for label, v in r['sides'].items():
            cost = ', a first run with an empty cache costs ' + ' and '.join(f'{m:.0f} M instructions under {n}' for n, m in sorted(v['empty_cache_minstr'].items()) if m) if v.get('empty_cache_minstr') else ''
            w(f'- Side `{label}`: {v["kind"]}, binary `{v["sha"]}`{cost}.')
        if r.get('commute'):
            w(f'- Side `commute`: binary `{r["commute"]}`.')
        if r.get('timed_engines'):
            w(f'- Timed: only {r["timed_engines"]} of the binaries.')
        if r.get('missing_tools'):
            w(f'- Programs missing on this machine: {", ".join(r["missing_tools"])}.')
    for e in res['ends']:
        w(f'- A run ended after {e["elapsed_s"]} s.')
    w(f'- {res["rows"]} rows, {res["admitted"]} admitted, {res["harness_errors"]} harness errors.')
    if res['refused']:
        w('- Refused: ' + ', '.join(f'{v} {k}' for k, v in res['refused'].items()) + '.')
    if 'official' not in res['baselines']:
        w('- **Against Arch\'s pdfTeX alone.** The official build ran no rows here, so every figure may flatter rstex: tests/bench/README.md, Baselines.')
    w('')
    w('Everything is rstex over the baseline: under 1 is fewer. `N / M` is how many of a figure\'s rows were identical to pdfTeX\'s. A row that differs is charged what it took plus the baseline, over the baseline. Figures are in user-mode instructions unless a line says otherwise, which are not the time.')
    w('')
    for tier, parts in res['tiers'].items():
        for part, p in parts.items():
            part_name = 'The private part, by id only' if part == 'private' else ('The hold-out, apart' if tier == 'holdout' else 'Public documents')
            w(f'## `{tier}`: {part_name}, {p["documents"]} documents')
            w('')
            write_part(w, p, res, part)
    if res.get('holdout_outside'):
        w('## Strata of the hold-out outside the interval of `standard`')
        w('')
        for x in res['holdout_outside']:
            w(f'- {x["figure"]}, {x["engine"]} against {x["baseline"]}, {x["stratum"]}: {x["holdout"]:.3f}, interval {x["standard_interval"][0]:.3f} to {x["standard_interval"][1]:.3f}.')
        w('')
    if res.get('true_share'):
        w('## `standard` by its weights against `full` on the same run')
        w('')
        for x in res['true_share']:
            w(f'- {x["figure"]}, {x["engine"]} against {x["baseline"]}: `standard` {x["standard"]:.3f}, interval {x["interval"][0]:.3f} to {x["interval"][1]:.3f}; `full` {x["full"]:.3f}{"" if x["inside"] else " **outside it**"}.')
        w('')
    if res.get('estimate'):
        write_estimate(w, res['estimate'])
    return '\n'.join(out) + '\n'


def write_part(w, p, res, part):
    if p['unavailable']:
        w('Rows that could not run: ' + ', '.join(f'{k} {v}' for k, v in sorted(p['unavailable'].items())) + '.')
        w('')
    marks = p['marks']
    w(f'Training overlap: {marks["trained_n"]} documents are trained on by `tools/pgo.py`, {marks["family_n"]} share a family with one' + (f': {", ".join(marks["trained"])} (trained); {", ".join(marks["family"])} (family).' if part == 'public' and (marks['trained'] or marks['family']) else '.'))
    w('')
    if p['wrong']:
        w('**Rows not identical to pdfTeX\'s.** They stay in M and are charged:')
        w('')
        for x in p['wrong']:
            w(f'- `{x["id"]}`, {x["figure"]}, {x["side"]}: {x["check"]}' + (f'; the oracle has {x["tex_errors"]} TeX errors on this document' if x.get('tex_errors') else ''))
        w('')
    if p['baseline_differs']:
        w('A baseline\'s own rows not identical to the reference: ' + ', '.join(f'{k} {v}' for k, v in sorted(p['baseline_differs'].items())) + '.')
        w('')
    # The table of all figures first.
    w('| Figure | Side | Against | N / M | Real documents, charged | 95% interval | Uncharged | Pooled by weight | Without PGO-marked rows | median | p90 | max |')
    w('|---|---|---|---:|---:|---|---:|---:|---:|---:|---:|---:|')
    for name, fig in p['figures'].items():
        for engine, by in fig['by'].items():
            for baseline in res['baselines']:
                tab = by.get(baseline)
                if not tab or 'real' not in tab:
                    continue
                r = tab['real']
                if not r['n']:
                    continue
                d = tab.get('distribution') or {}
                po = tab.get('pooled') or {}
                unm = po.get('without_pgo_marked') or {}
                w(f'| {name} | {engine} | {baseline} | {r["identical"]} / {r["n"]} | {num(r["charged"])} | {ci(r.get("interval"))} | {num(r["uncharged"])} | {num(po.get("value"))} | {num(unm.get("value"))} | {num(d.get("median"))} | {num(d.get("p90"))} | {num(d.get("max"))} |')
    w('')
    for name, fig in p['figures'].items():
        for engine, by in fig['by'].items():
            for baseline, tab in by.items():
                if baseline not in res['baselines'] or not tab.get('real'):
                    continue
                r = tab.get('tests') or tab.get('standins')
                if not r and not tab.get('real', {}).get('n'):
                    continue
                write_figure(w, name, fig['title'], engine, baseline, tab, res)
    if p['native_over_pgf']:
        w('The native diagram path over real PGF, on the same binary, documents that read `tikz-cd.sty`: ' + '; '.join(f'{k}: {v["native_over_pgf"]:.3f} over {v["n"]}, {v["identical_native"]} / {v["n"]} and {v["identical_pgf"]} / {v["n"]} identical' for k, v in p['native_over_pgf'].items()) + '.')
        w('')
    for k, v in p.get('side_stress', {}).items():
        w(f'A side table on how things scale, never pooled, {k.replace("|", " against ")}: ' + ', '.join(f'`{x["id"]}` {x["ratio"]:.3f}' + ('' if x['check'] in RIGHT else f' ({x["check"]})') for x in v) + '.')
        w('')
    if p['restored']:
        w('Restores, proved by the trace: ' + ', '.join(f'{k.replace("|", " ")} {v}' for k, v in p['restored'].items()) + '.' + (' The `warm` row of these is a cold run, and stays in the figure: ' + ', '.join(f'`{i}`' for i in p['warm_not_restored']) + '.' if p.get('warm_not_restored') else '') + (' A word in the body lost the checkpoint on: ' + ', '.join(f'`{i}`' for i in p['word_not_restored']) + '.' if p.get('word_not_restored') else ''))
        w('')
    if p['resume_records']:
        w('`resume`: ' + ', '.join(f'{v} {k}' for k, v in sorted(p['resume_records'].items())) + '.')
        w('')
    if p.get('reference_against_oracle'):
        w('`first`, the PDF of latexmk over Arch\'s against the oracle\'s: ' + ', '.join(f'{v} {k}' for k, v in sorted(p['reference_against_oracle'].items())) + '. Not judged: latexmk stops at a TeX error where the oracle\'s passes go on, and a biblatex document\'s oracle ran no biber.')
        w('')
    if p['edit_states']:
        w('Edits, rows of rstex: ' + ', '.join(f'{k.replace("|", " ")} {v}' for k, v in p['edit_states'].items()) + '.')
        w('')
    if p['synctex']:
        w('SyncTeX files against latexmk over Arch\'s, not counted in identical: ' + ', '.join(f'{v} {k}' for k, v in sorted(p['synctex'].items())) + '. On documents that read no `tikz-cd.sty`, where the native path\'s exception does not apply: ' + (', '.join(f'`{i}`' for i in p['synctex_differs_without_diagrams']) or 'none') + '.')
        w('')


def write_figure(w, name, title, engine, baseline, tab, res):
    r = tab['real']
    mark = ' (against Arch\'s alone)' if baseline == 'arch' and 'official' not in res['baselines'] else ''
    w(f'### {name}: {engine} against {baseline}{mark}')
    w('')
    w(title + '.')
    w('')
    if r['n']:
        w(f'Real documents: **{num(r["charged"])}** charged, {r["identical"]} / {r["n"]} identical, interval {ci(r.get("interval"))}, uncharged {num(r["uncharged"])}.')
        po = tab.get('pooled')
        if po:
            w(f'Pooled by weight over {po["n"]}: **{num(po["value"])}**, interval {ci(po.get("interval"))}' + (f'; without the rows `pgo` marks, over {po["without_pgo_marked"]["n"]}: {num(po["without_pgo_marked"]["value"])}, interval {ci(po["without_pgo_marked"].get("interval"))}' if po.get('without_pgo_marked') else '') + '.')
    for key, label in (('tests', 'Regression tests, a stratum of their own'), ('standins', 'Letter stand-ins, a stratum of their own')):
        x = tab.get(key)
        if x:
            w(f'{label}: {num(x["charged"])} charged, {x["identical"]} / {x["n"]} identical.')
    d = tab.get('distribution')
    if d:
        w(f'Distribution: min {num(d["min"])}, p25 {num(d["p25"])}, median {num(d["median"])}, p75 {num(d["p75"])}, p90 {num(d["p90"])}, max {num(d["max"])}.')
    if tab.get('worst'):
        w('Worst ten: ' + ', '.join(f'`{x["id"]}` {x["ratio"]:.3f}' + ('' if x['check'] in RIGHT else f' ({x["check"]})') + (f' [{x["pgo"]}]' if x.get('pgo') else '') for x in tab['worst']) + '.')
    a = tab.get('also') or {}
    if a.get('peak_rss'):
        w(f'Also: peak memory {num(a["peak_rss"])} (median {a["peak_rss_kb"] / 1024:.1f} MB and {a["peak_rss_kb_baseline"] / 1024:.1f} MB), page faults {num(a.get("page_faults"))}, PDF size {num(a.get("pdf_bytes"))}.')
    if tab.get('absent'):
        w('Without a figure: ' + ', '.join(f'{v} {k}' for k, v in sorted(tab['absent'].items())) + '.')
    c = tab.get('clock')
    if c:
        cyc = num(c['cycles']) if c.get('cycles') else c.get('cycles_withheld')
        control = f'control A/A {num(c["control"])}' if c.get('control') else 'no control in this round'
        right = f', {c["identical"]} / {c["n"]} identical' if 'identical' in c else ''
        w(f'Clock, charged, over {c["n"]} documents with rows marked for it{right}: paired cycles {cyc}, {control}, CPU {num(c["cpu"])}, wall {num(c["wall"])}.')
    if tab.get('strata'):
        w('')
        w('| Stratum | N / M | Charged | Interval | Uncharged |')
        w('|---|---:|---:|---|---:|')
        for s, x in tab['strata'].items():
            shown = ci(x.get('interval')) if x.get('interval') else ', '.join(f'`{r["id"]}` {r["ratio"]:.3f}' + ('' if r['right'] else ' (not identical)') + (f' [{r["pgo"]}]' if r.get('pgo') else '') for r in x.get('rows', ()))
            w(f'| {s} | {x["identical"]} / {x["n"]} | {num(x["charged"])} | {shown} | {num(x["uncharged"])} |')
    w('')


def write_estimate(w, est):
    w('## What a tier would cost on the PC, from this run')
    w('')
    w(est['how'])
    w('')
    w('| Tier | Scenario | Rows | Counted, CPU seconds | Timed, serial seconds |')
    w('|---|---|---:|---:|---:|')
    for t, rows in est['by_scenario'].items():
        for s, x in rows.items():
            w(f'| {t} | {s} | {x["rows"]} | {x["counted_s"]:.0f} | {x["timed_s"]:.0f} |')
    w('')
    w('| Tier | Counted, CPU hours | Over 14 cores, minutes | Timed, serial minutes | Together, minutes |')
    w('|---|---:|---:|---:|---:|')
    for t, x in est['total'].items():
        w(f'| {t} | {x["counted_s"] / 3600:.2f} | {x["counted_s"] / 14 / 60:.0f} | {x["timed_s"] / 60:.0f} | {(x["counted_s"] / 14 + x["timed_s"]) / 60:.0f} |')
    w('')
    other = est.get('at_rates_seen')
    if other:
        seen = est['rates_seen']
        w(f'The same at the rates this run saw per second of CPU, {seen["tex"] / 1e9:.1f} G for pdfTeX and {seen["rstex"] / 1e9:.1f} G for rstex, as the PC\'s oracle cache was made under a high -j, so its rate is on the low side. For planning only: nothing here says how fast either is.')
        w('')
        w('| Tier | Counted, CPU hours | Over 14 cores, minutes | Timed, serial minutes | Together, minutes |')
        w('|---|---:|---:|---:|---:|')
        for t, x in other.items():
            w(f'| {t} | {x["counted_s"] / 3600:.2f} | {x["counted_s"] / 14 / 60:.0f} | {x["timed_s"] / 60:.0f} | {(x["counted_s"] / 14 + x["timed_s"]) / 60:.0f} |')
        w('')


# ------------------------------------------------------------------------------------------------ what a tier costs
COUNTED_JOBS = 14
# A thinned plan cuts how many seconds of the oracle's pass a scenario goes up to.  A rule on what the oracle takes, so it cannot favour what rstex gains on: no result goes into it.
THIN = {'timed-5s': {'standard': {'timed': {'last-cold': 5.0, 'warm': 5.0, 'native': 5.0, 'hits': 2.0}}, 'holdout': {'timed': {'last-cold': 5.0, 'warm': 5.0}}, 'quick': {'timed': {'last-cold': 5.0, 'warm': 5.0}}}}


def thin_plan(plan, tier, name):
    """`plan` with the limits of a thinned plan over it."""
    if not name:
        return plan
    if name not in THIN:
        die(f'no thinned plan called {name}')
    res = {k: (dict(v) if isinstance(v, dict) else v) for k, v in plan.items()}
    for method, limits in THIN[name].get(tier, {}).items():
        res[method].update(limits)
    return res


def estimate(paths, set_path, local_path, engines=2, thin=None, timed_engines=None, observed=False):
    """From the rows of this run: how many instructions each scenario spends per instruction of the oracle's last pass, by rstex and by the baselines; then each tier's documents, under its plan, at the PC's rate.  A run
    of a side the rows have, and for what they have not, the same scenario's ratio over all documents.  Python around the runs is added per row, and the sleeps a seal takes are in the counted slots."""
    heads, rows, ends, jobs, errors = load_rows(paths)
    public, entries, private = load_set(set_path, local_path)
    both = dict(entries)
    both.update(private)
    rate = public['pc_rate_instr_per_ms']
    per = collections.defaultdict(lambda: {'rstex': 0.0, 'tex': 0.0, 'measured': 0.0, 'minstr': 0.0, 'docs': set(), 'rows': 0})
    kinds = {}
    for h in heads:
        kinds.update({k: v['kind'] for k, v in h['sides'].items()})
    n_engines = max(1, sum(1 for v in kinds.values() if v == 'rstex'))
    cycles = collections.defaultdict(lambda: [0, 0])
    for r in rows:
        if r['method'] != 'counted' or not r.get('instructions') or r['id'] not in both:
            continue
        e = both[r['id']]
        side_class = 'rstex' if kinds.get(r['side']) == 'rstex' else 'tex'
        x = per[r['scenario']]
        x[side_class] += r['instructions']
        x['rows'] += 1
        if side_class == 'rstex' and r.get('variant') != 'write':
            x['measured'] += r['instructions']
        if r['id'] not in x['docs']:
            x['docs'].add(r['id'])
            x['minstr'] += e['minstr'] * 1e6
        if r.get('cycles'):
            cycles[side_class][0] += r['instructions']
            cycles[side_class][1] += r['cycles']
    ipc = {k: (v[0] / v[1] if v[1] else None) for k, v in cycles.items()}
    # Instructions per second on the PC, from pdflatex's there, with rstex at its measured IPC against pdflatex's in the same conditions.
    speed = {'tex': rate * 1e3, 'rstex': rate * 1e3 * ((ipc.get('rstex') or 1) / (ipc.get('tex') or 1))}
    seen = {}
    for side_class in ('tex', 'rstex'):
        part = [r for r in rows if r['method'] == 'counted' and r.get('instructions') and r.get('cpu_ms') and (kinds.get(r['side']) == 'rstex') == (side_class == 'rstex') and r['side'] != 'commute']
        seen[side_class] = sum(r['instructions'] for r in part) / (sum(r['cpu_ms'] for r in part) / 1e3) if part else None
    if observed and seen['tex'] and seen['rstex']:
        speed = dict(seen)
    counted_jobs = [j for j in jobs if j.get('method') == 'counted']
    counted_rows = [r for r in rows if r['method'] == 'counted']
    worker_s = sum(j.get('seconds', 0) for j in counted_jobs)
    in_runs = sum((r.get('wall_ms') or 0) for r in counted_rows) / 1e3
    sleeps = sum(j.get('sleep_s', 0) for j in counted_jobs)
    n_rows = max(1, len(counted_rows))
    per_row = max(0.0, worker_s - in_runs - sleeps) / n_rows
    res = {'by_scenario': {}, 'total': {}, 'how': '', 'rates_seen': seen}
    plan_all = public['plan']
    res['how'] = (f'Instructions per instruction of the oracle\'s last pass, by scenario, from this run\'s counted rows, scaled to each document by its `minstr`; a side is {n_engines} rstex binaries as here, and the rest as here.  At {rate * 1e3 / 1e9:.2f} G instructions a second for pdflatex, the median of the PC\'s oracle cache, and '
                  f'rstex at {speed["rstex"] / 1e9:.2f} G by the ratio of IPC here ({num(ipc.get("rstex"), 2)} and {num(ipc.get("tex"), 2)}, on a machine under load).  {per_row:.2f} s of Python, the check and files per row, as measured here under load, so on the long side, and {sleeps:.0f} s of sleeps for seals in this run, scaled by rows.  Each document\'s `first` is scaled as a whole, so a tier of longer documents is an extrapolation.')
    timed_engines = engines if timed_engines is None else timed_engines
    for tier in ('quick', 'standard', 'holdout', 'full'):
        plan = thin_plan(plan_all[tier], tier, thin)
        docs = [e for e in both.values() if tier in e.get('tiers', ())]
        table = {}
        tc, tt = 0.0, 0.0
        for name in ('first', 'last-cold', 'warm', 'warm-word', 'hits', 'resume', 'edit', 'native', 'old-map'):
            x = per.get(name)
            limit = plan['counted'].get(name, 0)
            timed_limit = plan['timed'].get(name, 0)
            applicable = []
            for e in docs:
                t = e['minstr'] * 1e6 / rate / 1000
                if e.get('kind') == 'testfile' and name not in ('last-cold', 'warm'):
                    continue
                if name == 'edit' and not (tier == 'full' or e.get('edit')):
                    continue
                if name == 'native' and 'tikz-cd' not in e.get('features', ()):
                    continue
                applicable.append((e, t))
            count, timed, rows_here = 0.0, 0.0, 0
            source = x if x and x['minstr'] else None
            if name == 'old-map' and not source:
                source = {'tex': 2.0, 'rstex': 0.0, 'measured': 0.0, 'minstr': 1.0, 'rows': 2, 'docs': {1}}
            if not source:
                continue
            ktex = source['tex'] / source['minstr']
            krstex = source['rstex'] / source['minstr'] * (engines / n_engines)
            r_per_doc = source['rows'] / max(1, len(source['docs']))
            for e, t in applicable:
                instr = e['minstr'] * 1e6
                if limit is None or (limit and t <= limit) or (limit == 0 and False):
                    if name in plan['counted']:
                        count += instr * ktex / speed['tex'] + instr * krstex / speed['rstex'] + r_per_doc * per_row
                        rows_here += r_per_doc
                        if name in ('warm', 'warm-word'):
                            count += 2.1 * engines
                        if name == 'edit':
                            count += 2.2 + 2.1 * 0
                if name in plan['timed'] and (timed_limit is None or t <= timed_limit) and not (tier == 'full' and e.get('kind') == 'testfile'):
                    rounds = next((n for most, n in plan['rounds'] if most is None or t <= most), 1)
                    # Each rstex twice, and each pdfTeX once, a round, and what prepares a restore once.
                    k = source['measured'] / source['minstr'] * (timed_engines / n_engines)
                    tex_part = (instr * ktex / speed['tex']) if name == 'last-cold' else 0.0
                    timed += rounds * (2 * (instr * k / speed['rstex']) + tex_part)
                    if name == 'warm':
                        # A write for each binary, a seal for each, and one wait between them.
                        timed += instr * (source['rstex'] - source['measured']) / source['minstr'] * (timed_engines / n_engines) / speed['rstex'] + instr * k / speed['rstex'] + 2.1
            table[name] = {'rows': round(rows_here), 'counted_s': count, 'timed_s': timed}
            tc += count
            tt += timed
        # A format build on each side, and the diagrams, as measured.
        fmt = [r for r in rows if r['scenario'] == 'fmt' and r['method'] == 'counted' and r.get('instructions')]
        if fmt:
            f_tex = sum(r['instructions'] for r in fmt if kinds.get(r['side']) != 'rstex') / speed['tex']
            f_rstex = sum(r['instructions'] for r in fmt if kinds.get(r['side']) == 'rstex') / speed['rstex'] * (timed_engines / n_engines)
            fr = {'quick': 1, 'standard': 3, 'holdout': 1, 'full': 3}[tier]
            table['fmt'] = {'rows': (2 + timed_engines) * fr, 'counted_s': 0.0, 'timed_s': fr * (f_tex + f_rstex)}
            tt += table['fmt']['timed_s']
        dia = [r for r in rows if r['scenario'] == 'diagram-alone' and r['method'] == 'counted' and r.get('instructions')]
        if dia:
            n_here = max(1, len({r['id'] for r in dia}))
            per_diagram = sum(r['instructions'] / (speed['tex'] if r['side'] != 'commute' else speed['rstex']) for r in dia) / n_here
            d = public.get('diagrams', {})
            n_tier = len(d.get(tier) or ()) if tier != 'full' else d.get('usable', 0)
            if tier == 'full':
                table['diagram-alone'] = {'rows': n_tier, 'counted_s': n_tier * (per_diagram + 3 * per_row), 'timed_s': 0.0}
                tc += table['diagram-alone']['counted_s']
            else:
                table['diagram-alone'] = {'rows': n_tier, 'counted_s': 0.0, 'timed_s': n_tier * per_diagram}
                tt += table['diagram-alone']['timed_s']
        if tier == 'standard':
            pix = [e for e in docs if e.get('pixels')]
            x = per.get('edit')
            k = (x['rstex'] / x['minstr'] / max(1, x['rows'] / len(x['docs']) / 2)) if x and x['minstr'] else 0.3
            p = sum(8 * (e['minstr'] * 1e6 * k / speed['rstex']) * 3 * timed_engines + 4 * 0.2 for e in pix)
            table['pixels'] = {'rows': len(pix), 'counted_s': 0.0, 'timed_s': p}
            tt += p
        res['by_scenario'][tier] = table
        res['total'][tier] = {'counted_s': tc, 'timed_s': tt}
    return res


# ------------------------------------------------------------------------------------------------ main
def compact(x):
    """Five digits are more than a page can show, and all that the instruments say."""
    if isinstance(x, float):
        return float(f'{x:.5g}')
    if isinstance(x, dict):
        return {k: compact(v) for k, v in x.items()}
    if isinstance(x, (list, tuple)):
        return [compact(v) for v in x]
    return x


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('files', nargs='+')
    ap.add_argument('--set', default=os.path.join(HERE, 'set.json'))
    ap.add_argument('--set-local', default=os.path.join(HERE, 'set.local.json'))
    ap.add_argument('--tier')
    ap.add_argument('--resamples', type=int, default=10000)
    ap.add_argument('--md')
    ap.add_argument('--json')
    ap.add_argument('--draft', help='a line at the top: what this run is not')
    ap.add_argument('--estimate', action='store_true', help='add what each tier would cost on the PC, from this run')
    ap.add_argument('--engines', type=int, default=2, help='how many rstex binaries the estimate is for')
    ap.add_argument('--timed-engines', type=int, help='how many of them are timed, if not all')
    ap.add_argument('--thin', help='the estimate under a thinned plan')
    args = ap.parse_args()
    res = analyze(args.files, args.set, args.set_local, args.resamples, args.tier)
    if args.estimate:
        res['estimate'] = estimate(args.files, args.set, args.set_local, args.engines, args.thin, args.timed_engines)
        res['estimate']['at_rates_seen'] = estimate(args.files, args.set, args.set_local, args.engines, args.thin, args.timed_engines, observed=True)['total']
    text = markdown(res, args.draft)
    if args.md:
        with open(args.md, 'w') as f:
            f.write(text)
    if args.json:
        with open(args.json, 'w') as f:
            json.dump(compact(res), f, sort_keys=True, default=str, separators=(',', ':'))
    if not args.md:
        sys.stdout.write(text)


if __name__ == '__main__':
    main()
