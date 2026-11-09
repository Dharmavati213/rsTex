#!/usr/bin/env -S python3 -I
r"""Can the corpus runner pass while the output is wrong?  Each bug in bugs.py is run through the real `run.py compare`.

    python3 -I tests/corpus/mutate/matrix.py [--only REGEX] [--profile NAME]... [--out FILE]

Profiles are flag sets of `run.py compare`:

    defaults   no flag at all: what a person gets who types the README's first command
    bringup    last pass only, no L1, L2 or L6, the log noted but not gating, for the first months of an engine
    smoke      the flags of `run.py smoke`
    max        every level the runner has

The verdict for a bug under a profile is DETECTED, MISSED, or NOT-APPLIED.  Controls must come out identical.
Exit status: 0 when every bug is detected under `defaults` and the controls pass, 1 otherwise.
"""
import argparse
import concurrent.futures
import importlib.util
import json
import os
import re
import shutil
import stat
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(os.path.dirname(HERE)))
RUN = os.path.join(REPO, 'tests', 'corpus', 'run.py')
MANIFEST = os.path.join(REPO, 'tests', 'corpus', 'fixtures', 'mutation.json')
ROOT = os.path.join(REPO, 'target', 'corpus', 'mutate')
spec = importlib.util.spec_from_file_location('mutate_bugs', os.path.join(HERE, 'bugs.py'))
bugs = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bugs)

PROFILES = {
    'defaults': [],
    'bringup': ['--bringup'],
    'max': ['--mode', 'both', '--l6', '--box-log', 'always', '--gate-log', '--fls'],
    'smoke': ['--bringup', '--raster', 'none', '--box-log', 'never'],               # what run.py smoke does, by way of compare
}


def build_tfms():
    """The real cmss10 with one kern or one ligature changed, through tftopl and pltotf.  Not cmr10: the LaTeX format preloads
    that one, so a document run never opens its TFM and the changed file would be silently ignored."""
    src = subprocess.run(['kpsewhich', 'cmss10.tfm'], capture_output=True, text=True).stdout.strip()
    pl = subprocess.run(['tftopl', src], capture_output=True, text=True, errors='replace').stdout
    # cmss10's table: (LABEL C A) ... (KRN C V R -0.111112)  and (LABEL C f) (LIG C i O 14).  In the table the first such
    # kern is the pair A V; 0.000002 of the design size is about 1.3 sp at 10 pt.
    variants = {
        'kern-tfm': lambda s: s.replace('(KRN C V R -0.111112)', '(KRN C V R -0.122223)', 1),
        'kern-tfm-1sp': lambda s: s.replace('(KRN C V R -0.111112)', '(KRN C V R -0.111114)', 1),
        'ligature-tfm': lambda s: s.replace('(LIG C i O 14)\n', '', 1),
    }
    for bug, change in variants.items():
        new = change(pl)
        if new == pl:
            sys.exit(f'TFM mutation {bug} did not apply; the table of cmss10 is not what this script expects')
        d = os.path.join(ROOT, 'state', 'tfm', bug)
        os.makedirs(d, exist_ok=True)
        plf = os.path.join(d, 'cmss10.pl')
        with open(plf, 'w') as f:
            f.write(new)
        r = subprocess.run(['pltotf', plf, os.path.join(d, 'cmss10.tfm')], capture_output=True, text=True)
        if not os.path.isfile(os.path.join(d, 'cmss10.tfm')):
            sys.exit(f'pltotf failed for {bug}: {r.stdout} {r.stderr}')


def build_format():
    d = os.path.join(ROOT, 'state', 'format')
    os.makedirs(d, exist_ok=True)
    fmt = os.path.join(d, 'mutfmt.fmt')
    if not os.path.isfile(fmt):
        env = dict(os.environ, MKTEXFMT='0', MKTEXTFM='0', MKTEXPK='0', MKTEXMF='0', MKTEXTEX='0', SOURCE_DATE_EPOCH='1700000000', FORCE_SOURCE_DATE='1')
        subprocess.run(['pdflatex', '-ini', '-jobname=mutfmt', '-interaction=batchmode', '-output-directory=' + d,
                        r'&pdflatex \catcode`\@=11 \gdef\@gobblefour#1#2#3#4{wrong}\catcode`\@=12 \dump'], cwd=d, env=env, capture_output=True)
        if not os.path.isfile(fmt):
            sys.exit('could not build the mutated format')


def wrapper_for(bug):
    d = os.path.join(ROOT, 'wrappers')
    os.makedirs(d, exist_ok=True)
    path = os.path.join(d, bug)
    with open(path, 'w') as f:
        f.write(f'#!/bin/bash\nexport MUTATE_STATE={json.dumps(os.path.join(ROOT, "state"))}\n'
                f'exec python3 -I {json.dumps(os.path.join(HERE, "bugs.py"))} {bug} "$@"\n')
    os.chmod(path, os.stat(path).st_mode | stat.S_IXUSR)
    return path


def run_one(bug, profile):
    state = os.path.join(ROOT, 'state', bug)
    shutil.rmtree(state, ignore_errors=True)
    entry = bugs.ENTRY[bug]
    run_id = f'{bug}-{profile}'
    runs = os.path.join(ROOT, 'runs')
    shutil.rmtree(os.path.join(runs, run_id), ignore_errors=True)
    argv = [sys.executable, '-I', RUN, 'compare', '--engine', wrapper_for(bug), '--manifest', MANIFEST,
            '--filter', '^' + re.escape(entry) + '$', '--runs-dir', runs, '--run-id', run_id, '-j', '1'] + PROFILES[profile]
    p = subprocess.run(argv, capture_output=True, text=True, errors='replace')
    results = []
    path = os.path.join(runs, run_id, 'results.jsonl')
    if os.path.isfile(path):
        with open(path) as f:
            results = [json.loads(line) for line in f if line.strip()]
    applied = os.path.isfile(os.path.join(state, 'applied'))
    out = {'bug': bug, 'kind': bugs.KIND.get(bug, 'found in review'), 'entry': entry, 'profile': profile, 'exit': p.returncode,
           'applied': applied, 'modes': {}}
    for r in results:
        fd = r.get('first_divergence') or {}
        lv = r.get('levels', {})
        out['modes'][r['mode']] = {'outcome': r['outcome'], 'first': fd.get('text'), 'all': fd.get('all'), 'box': fd.get('box_log'),
                                   'L4_L5': lv.get('L4_L5', {}).get('status'), 'L6': lv.get('L6', {}).get('status'),
                                   'L2': lv.get('L2', {}).get('status'), 'L1': lv.get('L1', {}).get('status'), 'L7': bool(lv.get('L7', {}).get('diff_pages')),
                                   'log': lv.get('log', {}).get('status'), 'note': r.get('log_note')}
    if bug.startswith('identity'):
        out['verdict'] = 'CONTROL-OK' if results and all(r['outcome'] == 'identical' for r in results) else 'CONTROL-FAILED'
    elif not results:
        out['verdict'] = 'NO-RESULT'
    elif not applied and not any(r['outcome'] == 'candidate-crash' for r in results):
        out['verdict'] = 'NOT-APPLIED'
    elif any('did not apply' in (r.get('first_divergence') or {}).get('text', '') for r in results):
        out['verdict'] = 'NOT-APPLIED'
    else:
        out['verdict'] = 'DETECTED' if any(r['outcome'] != 'identical' for r in results) else 'MISSED'
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--only', help='regular expression over bug names')
    ap.add_argument('--profile', action='append', choices=sorted(PROFILES))
    ap.add_argument('--out', default=os.path.join(ROOT, 'matrix.json'))
    ap.add_argument('-j', '--jobs', type=int, default=6)
    args = ap.parse_args()
    os.makedirs(ROOT, exist_ok=True)
    build_tfms()
    build_format()
    names = [b for b in bugs.ENTRY if not args.only or re.search(args.only, b)]
    profiles = args.profile or ['defaults', 'max']

    def one_row(bug):
        row = {'bug': bug}
        for prof in profiles:
            row[prof] = run_one(bug, prof)
        print(f'{bug:<28}' + ''.join(f' {prof}={row[prof]["verdict"]:<12}' for prof in profiles), flush=True)
        return row

    with concurrent.futures.ThreadPoolExecutor(args.jobs) as ex:
        table = list(ex.map(one_row, names))
    os.makedirs(os.path.dirname(args.out), exist_ok=True)
    with open(args.out, 'w') as f:
        json.dump(table, f, indent=1)
    print()
    print(f'{"kind":<28} {"bug":<26} ' + ' '.join(f'{p:<12}' for p in profiles) + 'caught by, under ' + profiles[0])
    for row in table:
        first = row[profiles[0]]
        where = '; '.join(f'{m}: {(v["first"] or v["outcome"])[:70]}' for m, v in first['modes'].items() if v['outcome'] != 'identical')
        print(f'{first["kind"]:<28} {row["bug"]:<26} ' + ' '.join(f'{row[p]["verdict"]:<12}' for p in profiles) + where)
    missed = [row['bug'] for row in table if row[profiles[0]]['verdict'] not in ('DETECTED', 'CONTROL-OK')]
    detected = sum(row[profiles[0]]['verdict'] == 'DETECTED' for row in table)
    real = sum(not row['bug'].startswith('identity') for row in table)
    print(f'\n{detected} of {real} bugs detected under {profiles[0]}; {len(missed)} missed or not applied: {", ".join(missed) or "none"}')
    print(f'details: {args.out}')
    return 1 if missed else 0


if __name__ == '__main__':
    sys.exit(main())
