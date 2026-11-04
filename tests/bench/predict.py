#!/usr/bin/env -S python3 -I
"""predict.py: does the `standard` tier predict documents it does not hold?  A check of the weights and strata of tests/bench/select.py, by a fresh random sample of the real documents of `full`.

    predict.py draw [--n 300] [--seed S] [--out target/bench/predict/set.json]          a set file for tools/prof/benchset.py, of real documents of `full` that are not in `standard`
    predict.py compare --standard FILE --sample FILE [--sample-set PATH] [--engine plain] [--baseline arch] [--scenario last-cold,warm]     the tables, from rows of benchset.py

Run the sample as

    benchset.py run --tier standard --set target/bench/predict/set.json --no-private --engine plain=BIN --baselines arch --scenarios last-cold,warm --method counted --out target/bench/predict/rows.jsonl

The tier's name is only how benchset.py finds its plan: the file holds these documents under it.  The hold-out and its reserve are not sampled, so that this tells nothing of them: their families are measured once.

**How the two are held against each other.**  `standard` by its weights estimates the mean of `log ratio` over `full`, each document at its family's mass.  So does the sample, by the same mass, as it is drawn evenly over documents.  An interval comes from resampling families, the unit that is independent, and
the sample's own interval is printed, since a sample of 300 is not exact either.  For a stratum the report's own figure is compared: the geometric mean of a tier's documents in it, without weights.

This reads ratios of an engine, which select.py must never do.  It is a check after the fact: `weight` and the strata can move only by a reason that holds without the number, and README.md says what was done.  It prints public ids and numbers, and reads no private row.
"""
import os
import sys

# select.py in this directory is not the standard library's select.
sys.path[:] = [p for p in sys.path if os.path.abspath(p or os.curdir) != os.path.dirname(os.path.abspath(__file__))]

import argparse  # noqa: E402
import collections  # noqa: E402
import hashlib  # noqa: E402
import importlib.util  # noqa: E402
import json  # noqa: E402
import math  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
SEED = 'rstex-bench-predict-1'
FAMILY_MASS = 12
RESAMPLES = 4000


def load_report():
    spec = importlib.util.spec_from_file_location('bench_report_for_predict', os.path.join(HERE, 'report.py'))
    mod = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = mod
    spec.loader.exec_module(mod)
    return mod


report = load_report()


def is_real(e):
    return e['kind'] != 'testfile' and not e.get('synthetic') and e.get('family') != 'ladder'


def draw_set(public, n, seed):
    """`n` real documents of `full`, not of `standard`, by a hash of the seed and the id.  So the same on any machine, and nothing but ids goes in."""
    pool = sorted((e for e in public['entries'] if 'full' in e['tiers'] and 'standard' not in e['tiers'] and is_real(e)), key=lambda e: hashlib.sha256(f'{seed}\0{e["id"]}'.encode()).hexdigest())
    chosen = pool[:n]
    entries = [{k: v for k, v in e.items() if k not in ('edit', 'pixels', 'weight', 'entered')} | {'tiers': ['standard']} for e in sorted(chosen, key=lambda e: e['id'])]
    plan = {'counted': {'last-cold': None, 'warm': None, 'warm-word': None}, 'timed': {}, 'rounds': [[None, 1]]}
    return {'schema': 1, 'seed': seed, 'part': 'predict', 'pool': len(pool), 'plan': {'standard': plan}, 'tree': public['tree'], 'pc_rate_instr_per_ms': public['pc_rate_instr_per_ms'], 'entries': entries}


def family_masses(entries):
    full = [e for e in entries if 'full' in e['tiers'] and is_real(e)]
    sizes = collections.Counter(e['family'] for e in full)
    return {f: min(1.0, FAMILY_MASS / k) for f, k in sizes.items()}


def ratios(paths, figure, engine, baseline, entries):
    """{id: charged ratio}, by report.py's own pairing, so that this cannot differ from what a report would print."""
    _, rows, _, _, _ = report.load_rows(paths)
    t = report.Tables(rows)
    pairs, absent = report.pair_rows(t, sorted(entries), figure, engine, baseline)
    return {p[0]: p[1] for p in pairs}, dict(absent), sum(1 for p in pairs if p[2] is None)


def weighted(rows):
    """rows: (family, weight, log ratio)."""
    total = sum(w for _, w, _ in rows)
    return sum(w * x for _, w, x in rows) / total if total else None


def family_interval(rows, resamples, *labels):
    """The weighted mean's interval, resampling families, widened as report.py's are."""
    groups = collections.defaultdict(list)
    for f, w, x in rows:
        groups[f].append((w, x))
    prepared = [(sum(w * x for w, x in g), sum(w for w, _ in g)) for _, g in sorted(groups.items())]
    if len(prepared) < 2:
        return None
    rng = report.rng_for(*labels)
    means = []
    for _ in range(resamples):
        picks = rng.choices(prepared, k=len(prepared))
        total = sum(p[1] for p in picks)
        if total:
            means.append(sum(p[0] for p in picks) / total)
    means.sort()
    return report.widened(means, weighted(rows), len(prepared))


def line(name, rows, *labels):
    m = weighted(rows)
    if m is None:
        return {'name': name, 'n': 0}
    ci = family_interval(rows, RESAMPLES, name, *labels)
    return {'name': name, 'n': len(rows), 'families': len({f for f, _, _ in rows}), 'value': math.exp(m), 'interval': ci}


def show(x):
    if not x.get('n'):
        return f'{x["name"]:<46} no rows'
    ci = f'{x["interval"][0]:.3f} to {x["interval"][1]:.3f}' if x.get('interval') else 'one family'
    return f'{x["name"]:<46} n {x["n"]:>4}, {x["families"]:>4} families: {x["value"]:.4f}  [{ci}]'


def inside(value, interval):
    return interval is not None and interval[0] <= value <= interval[1]


def compare(args):
    with open(os.path.join(HERE, 'set.json')) as f:
        public = json.load(f)
    entries = {e['id']: e for e in public['entries']}
    mass = family_masses(public['entries'])
    with open(args.sample_set) as f:
        sample_set = json.load(f)
    sample_ids = {e['id']: entries[e['id']] for e in sample_set['entries']}
    old = {}
    if args.old_set:
        with open(args.old_set) as f:
            old = {e['id']: e for e in json.load(f)['entries']}
    std = {i: e for i, e in entries.items() if 'standard' in e['tiers'] and e['kind'] != 'testfile' and not e.get('synthetic')}
    figures = {f[0]: f for f in report.FIGURES}
    for name in args.scenario.split(','):
        figure = figures[name]
        a, absent_a, wrong_a = ratios([args.standard], figure, args.engine, args.baseline, std)
        b, absent_b, wrong_b = ratios(args.sample, figure, args.engine, args.baseline, sample_ids)
        print(f'\n{name}, {args.engine} over {args.baseline}.  standard: {len(a)} of {len(std)} rows, {wrong_a} charged, not paired {absent_a}.  sample: {len(b)} of {len(sample_ids)} rows, {wrong_b} charged, not paired {absent_b}.')
        out = []
        out.append(line('sample, documents even', [(sample_ids[i]['family'], 1.0, math.log(r)) for i, r in b.items()], name, 'even'))
        sample_mass = line('sample, by family mass', [(sample_ids[i]['family'], mass.get(sample_ids[i]['family'], 1.0), math.log(r)) for i, r in b.items()], name, 'mass')
        out.append(sample_mass)
        out.append(line('standard, its documents as they are', [(std[i]['family'], 1.0, math.log(r)) for i, r in a.items()], name, 'std-even'))
        if old:
            out.append(line('standard, weights before the fix', [(std[i]['family'], old[i]['weight'], math.log(r)) for i, r in a.items() if old.get(i, {}).get('weight')], name, 'std-old'))
        current = line('standard, by weight', [(std[i]['family'], std[i]['weight'], math.log(r)) for i, r in a.items() if std[i].get('weight')], name, 'std-new')
        # The interval a report prints: families resampled within cells.
        current['interval'] = report.pooled_interval([(std[i]['kind'] + '/' + std[i]['size'], std[i]['family'], std[i]['weight'], r) for i, r in a.items() if std[i].get('weight')], RESAMPLES, name, 'std-new-cells')
        out.append(current)
        for x in out:
            print(show(x))
        if sample_mass.get('n') and current.get('n'):
            print(f'standard by weight over the sample by mass: {current["value"] / sample_mass["value"]:.4f}.  The sample\'s figure inside standard\'s interval: {inside(sample_mass["value"], current["interval"])}; standard\'s inside the sample\'s: {inside(current["value"], sample_mass["interval"])}')
            # All of what the rows have of `full`, standard's documents and the sample's, each at its family's mass.  If the sample is all of `full` outside `standard`, this is `full`, with no error of a draw.
            whole = line('both together, by family mass', [(std[i]['family'], mass.get(std[i]['family'], 0), math.log(r)) for i, r in a.items() if std[i]['family'] in mass] + [(sample_ids[i]['family'], mass.get(sample_ids[i]['family'], 1.0), math.log(r)) for i, r in b.items()], name, 'whole')
            print(show(whole))
            print(f'standard by weight over both together: {current["value"] / whole["value"]:.4f}; inside its interval: {inside(whole["value"], current["interval"])}')
            if old:
                old_value = math.exp(weighted([(std[i]['family'], old[i]['weight'], math.log(r)) for i, r in a.items() if old.get(i, {}).get('weight')]))
                print(f'weights before the fix over both together: {old_value / whole["value"]:.4f}')
        # By stratum: documents even, as a stratum's line was, and each side at its mass.
        strata = collections.defaultdict(lambda: ([], [], [], []))
        population = [e for e in public['entries'] if 'full' in e['tiers'] and is_real(e)]
        members = collections.defaultdict(list)
        for i in a:
            for s in report.stratum_of(std[i]):
                members[s].append(std[i])
        for s, rows in members.items():
            w = report.stratum_weights(s, rows, population, mass, lambda i: std[i].get('weight'))
            for e in rows:
                strata[s][0].append((e['family'], 1.0, math.log(a[e['id']])))
                strata[s][2].append((e['family'], w[e['id']], math.log(a[e['id']])))
        for i, r in b.items():
            for s in report.stratum_of(sample_ids[i]):
                strata[s][1].append((sample_ids[i]['family'], 1.0, math.log(r)))
                strata[s][3].append((sample_ids[i]['family'], mass.get(sample_ids[i]['family'], 1.0), math.log(r)))
        print(f'{"stratum":<28}{"standard n,fam":>15}{"even":>7}{"at mass":>9}{"its interval":>16}{"sample n,fam":>14}{"even":>7}{"at mass":>9}{"its interval":>16}   standard\'s interval holds the sample at mass; even means inside')
        outside_mass = outside_even = compared = 0
        for s, (x, y, xw, yw) in sorted(strata.items()):
            if not x or not y:
                continue
            xw = [t for t in xw if t[1]]
            ex, ey = math.exp(weighted(x)), math.exp(weighted(y))
            mx, my = math.exp(weighted(xw)) if xw else float('nan'), math.exp(weighted(yw))
            ci_x = family_interval(xw, RESAMPLES, 'predict-m', name, s) if len({f for f, _, _ in xw}) >= report.FAMILIES_MIN and len(xw) >= report.INTERVAL_MIN else None
            ci_e = report.bootstrap([math.exp(v) for _, _, v in x], RESAMPLES, 'predict-f', name, s, units=[f for f, _, _ in x])
            ci_y = family_interval(yw, RESAMPLES, 'predict-g', name, s) if len({f for f, _, _ in yw}) >= report.FAMILIES_MIN and len(yw) >= report.INTERVAL_MIN else None
            mark = ''
            if ci_x and ci_e and len(y) >= report.INTERVAL_MIN:
                compared += 1
                outside_mass += not inside(my, ci_x)
                outside_even += not inside(ey, ci_e)
                mark = f'{inside(my, ci_x)!s:<6}{inside(ey, ci_e)!s:<6}'
            fmt = lambda c: f'{c[0]:.3f}-{c[1]:.3f}' if c else ''      # noqa: E731
            print(f'{s:<28}{len(x):>9},{len({f for f, _, _ in x}):>4}{ex:>8.3f}{mx:>9.3f}{fmt(ci_x):>16}{len(y):>9},{len({f for f, _, _ in y}):>4}{ey:>7.3f}{my:>9.3f}{fmt(ci_y):>16}   {mark}')
        print(f'strata with an interval on standard and {report.INTERVAL_MIN} or more in the sample: {compared}; outside standard\'s interval, at mass {outside_mass}, documents even {outside_even}.  The sample has its own error: where it has an interval too, z is their difference over both errors')
        for s, (x, y, xw, yw) in sorted(strata.items()):
            xw = [t for t in xw if t[1]]
            if not xw or not yw or len({f for f, _, _ in xw}) < report.FAMILIES_MIN or len({f for f, _, _ in yw}) < report.FAMILIES_MIN or len(xw) < report.INTERVAL_MIN or len(yw) < report.INTERVAL_MIN:
                continue
            ca, cb = family_interval(xw, RESAMPLES, 'predict-m', name, s), family_interval(yw, RESAMPLES, 'predict-g', name, s)
            se = math.hypot(math.log(ca[1] / ca[0]), math.log(cb[1] / cb[0])) / (2 * 1.96)
            print(f'  {s:<28} standard over sample {math.exp(weighted(xw) - weighted(yw)):.3f}, z {(weighted(xw) - weighted(yw)) / se:+.2f}')


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest='cmd', required=True)
    d = sub.add_parser('draw')
    d.add_argument('--n', type=int, default=300)
    d.add_argument('--seed', default=SEED)
    d.add_argument('--out', default=os.path.join(REPO, 'target', 'bench', 'predict', 'set.json'))
    c = sub.add_parser('compare')
    c.add_argument('--standard', required=True)
    c.add_argument('--sample', required=True, nargs='+')
    c.add_argument('--sample-set', default=os.path.join(REPO, 'target', 'bench', 'predict', 'set.json'))
    c.add_argument('--old-set', help='an earlier set.json, for what its weights gave')
    c.add_argument('--engine', default='plain')
    c.add_argument('--baseline', default='arch')
    c.add_argument('--scenario', default='last-cold,warm')
    args = ap.parse_args()
    if args.cmd == 'draw':
        with open(os.path.join(HERE, 'set.json')) as f:
            public = json.load(f)
        doc = draw_set(public, args.n, args.seed)
        os.makedirs(os.path.dirname(args.out), exist_ok=True)
        with open(args.out, 'w') as f:
            json.dump(doc, f, sort_keys=True)
        print(f'{len(doc["entries"])} of {doc["pool"]} real documents of `full` outside `standard`, seed {args.seed}: {sum(e["minstr"] for e in doc["entries"]) / 1e3:.0f} G instructions a pass')
    else:
        compare(args)


if __name__ == '__main__':
    main()
