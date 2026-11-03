#!/usr/bin/env -S python3 -I
"""Regenerate target/corpus/gen/ and tests/corpus/manifests/coverage.json.

    python3 -I tests/corpus/gen/all.py [--only pkg,font,prim,fuzz,stress] [--seeds N] [--jobs N]

Deterministic: the same installed TeX Live gives the same documents and the same manifest.  Files that come out with the same
bytes keep their mtime, so run.py's oracle cache stays valid.  The fonts part compiles each mapped font once with the oracle to
find the ones that kill a document, and keeps the answer in target/corpus/gen/font/plan.json.
"""
import argparse
import os
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common  # noqa: E402

STANDARD_TOTAL = 200
# How the 200 standard entries are spread.  What is left over goes to full.  The rest of each category is chosen by even stride
# over the sorted list, so nothing is tuned; the named packages are the ones real documents load.
STANDARD_PACKAGES = """amsmath amssymb amsthm mathtools graphicx xcolor hyperref geometry microtype babel enumitem booktabs tikz pgfplots fontenc inputenc
lmodern listings siunitx biblatex natbib cleveref caption subcaption longtable multirow array tabularx fancyhdr titlesec xparse expl3 etoolbox calc ifthen
url float wrapfig""".split()
SPREAD = (('pkg', 40), ('cls', 15), ('babel-option', 5), ('babel-provide', 8), ('babel-ini', 7), ('font:table', 4), ('font:mapline', 2), ('font:notfm', 1),
          ('font:stub', 5), ('font:expansion', 3), ('prim', 45), ('prim-value', 5), ('prim-bare', 10), ('prim-fatal', 15), ('prim-ini', 5), ('fuzz', 25), ('stress', 5))


def category(e):
    return e['id'].split('/')[1]


def group_key(e):
    """The font documents come in kinds of very different size, so the spread goes by kind."""
    cat = category(e)
    if cat == 'font':
        return 'font:' + e['id'].split('/')[2].split('-')[0]
    return cat


def assign_tiers(entries):
    groups = {}
    for e in entries:
        e['tier'] = 'full'
        groups.setdefault(group_key(e), []).append(e)
    chosen = 0
    for cat, want in SPREAD:
        items = groups.get(cat, [])
        cheap = [e for e in items if not e.get('expensive')]
        picked = []
        if cat == 'pkg':
            named = {f'coverage/pkg/{n}' for n in STANDARD_PACKAGES}
            picked = [e for e in cheap if e['id'] in named]
            rest = [e for e in cheap if e['id'] not in named]
            picked += common.stride_sample(rest, max(0, want - len(picked)))
        else:
            picked = common.stride_sample(cheap, want)
        for e in picked:
            e['tier'] = 'standard'
        chosen += len(picked)
    for e in entries:
        e.pop('expensive', None)
    return chosen


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--only', help='comma-separated parts, default all; other parts are carried over from the existing manifest')
    ap.add_argument('--seeds', type=int, default=2000)
    ap.add_argument('--jobs', type=int, default=8)
    args = ap.parse_args()
    parts = set(args.only.split(',')) if args.only else {'pkg', 'font', 'prim', 'fuzz', 'stress'}
    entries, exclusions, counts = [], [], {}
    old = {}
    if args.only and os.path.isfile(common.MANIFEST):
        import json
        with open(common.MANIFEST) as f:
            old = json.load(f)
    t0 = time.time()

    def carry(cats):
        for e in old.get('entries', []):
            if category(e) in cats:
                entries.append(dict(e))

    if 'pkg' in parts:
        import packages
        en, ex, co = packages.generate()
        entries += en
        exclusions += ex
        counts.update(co)
    else:
        carry({'pkg', 'cls', 'babel-option', 'babel-provide', 'babel-ini'})
    if 'font' in parts:
        import fonts
        en, ex, co = fonts.generate(args.jobs)
        entries += en
        exclusions += ex
        counts.update(co)
    else:
        carry({'font'})
    if 'prim' in parts:
        import primitives
        en, ex, co = primitives.generate()
        entries += en
        exclusions += ex
        counts.update(co)
    else:
        carry({'prim', 'prim-bare', 'prim-value', 'prim-fatal', 'prim-ini'})
    if 'fuzz' in parts:
        import fuzz
        en, ex, co = fuzz.generate(args.seeds)
        entries += en
        exclusions += ex
        counts.update(co)
    else:
        carry({'fuzz'})
    if 'stress' in parts:
        import stress
        en, ex, co = stress.generate()
        entries += en
        exclusions += ex
        counts.update(co)
    else:
        carry({'stress'})
    if args.only:
        exclusions = old.get('exclusions', []) if not exclusions else exclusions + [x for x in old.get('exclusions', []) if x not in exclusions]
    chosen = assign_tiers(entries)
    manifest = {
        'source': 'coverage',
        'description': 'Generated tests that reach everything instead of sampling: every package, class and babel language, every mapped font, '
                       'every primitive with probes and error paths, a seeded differential fuzzer, and scaling stress cases. Regenerate with '
                       'tests/corpus/gen/all.py; the documents live in target/corpus/gen/ and are not committed. An oracle failure on these entries is '
                       'itself a test (judge_failure): the candidate must fail with the same log.',
        'defaults': {'passes': 1, 'gate_log': True, 'judge_failure': True, 'timeout': 60},
        'exclusions': exclusions,
        'entries': [],
    }
    for e in entries:
        if 'mode' not in e and category(e).startswith(('prim', 'font', 'fuzz', 'stress')):
            e['mode'] = 'plain'
        manifest['entries'].append(e)
    # One entry per line: a diff of a regeneration stays readable, and the file stays small.
    import json
    head = {k: manifest[k] for k in ('source', 'description', 'defaults', 'exclusions')}
    text = json.dumps(head, indent=1)[:-2] + ',\n "entries": [\n' + ',\n'.join('  ' + json.dumps(e, sort_keys=True) for e in manifest['entries']) + '\n ]\n}\n'
    tmp = common.MANIFEST + '.tmp'
    with open(tmp, 'w') as f:
        f.write(text)
    os.replace(tmp, common.MANIFEST)
    print(f'{len(entries)} entries, {chosen} standard, {len(exclusions)} bounds stated, {time.time() - t0:.1f} s')
    for k in sorted(counts):
        print(f'  {k}: {counts[k]}')


if __name__ == '__main__':
    main()
