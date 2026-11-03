#!/usr/bin/env -S python3 -I
r"""The tables in the last section of tests/corpus/README.md, from what the runner has recorded.

    python3 -I tests/corpus/table.py [--compare RUN] [--extra-compare RUN]...     # a run name under target/corpus/runs, default the latest compare

Reads the manifests, the cached oracles, target/corpus/audit/substance.json and one compare run.  Compiles nothing.  Prints Markdown.
Ids of the owner's documents never appear: those sources are counted, and the ids listed are public ones.
"""
import argparse
import collections
import importlib.util
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location('corpus_run', os.path.join(HERE, 'run.py'))
run = importlib.util.module_from_spec(spec)
spec.loader.exec_module(run)
PRIVATE = ('user', 'local', 'cd-local')
ORDER = ['ladder', 'user', 'local', 'public-stacks', 'public-books', 'public-notes', 'public-manuals', 'public-knuth', 'public-latex2e-docs', 'public-latex3-docs',
         'public-latex2e-tests', 'public-latex3-tests', 'public-package-tests', 'arxiv', 'coverage', 'coverage-extra', 'cd', 'cd-local']


def table(head, rows, align=None):
    align = align or ['---'] + ['---:'] * (len(head) - 1)
    print('| ' + ' | '.join(head) + ' |')
    print('|' + '|'.join(align) + '|')
    for r in rows:
        print('| ' + ' | '.join(str(x) for x in r) + ' |')
    print()


def current_meta(e):
    base = run.oracle_base(e)
    prefix = run.config_hash(e) + '-'
    best = None
    try:
        names = os.listdir(base)
    except OSError:
        return None
    for n in names:
        p = os.path.join(base, n, 'meta.json')
        if n.startswith(prefix) and os.path.isfile(p) and (best is None or os.path.getmtime(p) > os.path.getmtime(best)):
            best = p
    return run.read_json(best) if best else None


def fmt_s(ms):
    s = ms / 1000
    return f'{s:,.0f} s' if s < 3600 else f'{s / 3600:,.1f} h'


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--compare')
    ap.add_argument('--extra-compare', action='append', default=[], help='a run that holds results the main run lacks, such as a retry of one entry; it overrides by (id, mode)')
    args = ap.parse_args()
    entries, _, _ = run.load_manifests(None)
    audit = {}
    try:
        audit = run.read_json(run.SUBSTANCE)['entries']
    except (OSError, ValueError, KeyError):
        print('no audit: run tests/corpus/audit.py', file=sys.stderr)
    sources = collections.OrderedDict((s, collections.Counter()) for s in ORDER)
    tiers = collections.OrderedDict((t, collections.Counter()) for t in run.TIERS)
    metas = {}
    for e in entries:
        c = sources.setdefault(e['source'], collections.Counter())
        c['entries'] += 1
        c[e['tier']] += 1
        tc = tiers[e['tier']]
        tc['entries'] += 1
        if e.get('excluded'):
            c['excluded'] += 1
            tc['excluded'] += 1
            continue
        m = current_meta(e)
        metas[e['id']] = m
        if m is None:
            c['unprepared'] += 1
            tc['unprepared'] += 1
            continue
        cls = run.effective_class(m, e)
        key = {'normal': 'comparable', 'oracle-fails': 'oracle-fails', 'nondeterministic': 'nondeterministic'}[cls]
        c[key] += 1
        tc[key] += 1
        tc['cpu_all'] += sum(m.get('pass_cpu_ms') or [0])
        tc['cpu_last'] += (m.get('pass_cpu_ms') or [0])[-1] if m.get('pass_cpu_ms') else 0
        if cls == 'normal':
            log = m.get('log') or {}
            pages = log.get('pages')
            if pages is None and m.get('pdf'):        # a cached count older than log_stats' handling of a wrapped line
                pages = run.log_stats(os.path.join(run.oracle_base(e), m['key'], 'final', m['jobname'] + '.log')).get('pages')
            pages = pages or 0
            c['pages'] += pages
            tc['pages'] += pages
            a = audit.get(e['id'])
            verdict = a.get('verdict') if a and a.get('oracle_key') == m['key'] else 'not audited'
            bucket = {'substantive': 'substantive', 'vacuous': 'vacuous', 'duplicate': 'duplicate'}.get(verdict, 'not audited')
            c[bucket] += 1
            tc[bucket] += 1
    total = collections.Counter()
    rows = []
    for s, c in sources.items():
        if not c['entries']:
            continue
        total.update(c)
        rows.append([f'`{s}`', c['entries'], c['smoke'], c['standard'], c['full'], c['excluded'], c['oracle-fails'], c['nondeterministic'], c['comparable'], c['vacuous'], c['duplicate'], c['substantive'],
                     f"{c['pages']:,}"])
    rows.append(['**total**', total['entries'], total['smoke'], total['standard'], total['full'], total['excluded'], total['oracle-fails'], total['nondeterministic'], total['comparable'], total['vacuous'],
                 total['duplicate'], total['substantive'], f"{total['pages']:,}"])
    table(['Source', 'Entries', 'smoke', 'standard', 'full', 'Excluded', 'Oracle fails', 'Nondet.', 'Comparable', 'Vacuous', 'Duplicates', 'Substantive', 'Pages'], rows)
    if total['unprepared'] or total['not audited']:
        print(f'{total["unprepared"]} entries have no oracle for this checkout; {total["not audited"]} comparable entries are not audited.\n')

    rows = []
    for t, c in tiers.items():
        rows.append([t, c['entries'], c['comparable'], c['substantive'], f"{c['pages']:,}", fmt_s(c['cpu_all'] * 2), fmt_s(c['cpu_last'])])
    table(['Tier, on its own', 'Entries', 'Comparable', 'Substantive', 'Pages', 'Oracle CPU to prepare it (two clean compiles of each)', 'Oracle CPU, one last pass of each'], rows)

    moving = collections.defaultdict(list)
    for e in entries:
        m = metas.get(e['id'])
        if not m or run.effective_class(m, e) != 'normal':
            continue
        det = m.get('determinism') or {}
        if det.get('log_equal') is False:
            moving['the log differs between two clean oracle runs, so it does not gate'].append(e)
        if det.get('log_strict_equal') is False:
            moving['the whole log differs between two clean oracle runs, so only its l3build form gates'].append(e)
        if det.get('pdf_bytes_equal_except_id') is False:
            moving['the PDF bytes differ between two clean oracle runs, and agree once normalized'].append(e)
        if '\\pdfsetrandomseed' in e['prelude']:
            moving['the manifest pins the random seed with \\pdfsetrandomseed, or it would be nondeterministic'].append(e)
    print('Oracle classified nondeterministic: ' + str(sum(c['nondeterministic'] for c in sources.values())))
    for why, es in moving.items():
        public = [e['id'] for e in es if e['source'] not in PRIVATE]
        print(f'- {len(es)} entries: {why}' + (': ' + ', '.join(f'`{i}`' for i in public) if len(public) <= 30 else f'; first public: `{public[0]}`' if public else ''))
    print()

    root = run.DEFAULT_RUNS
    name = args.compare or (run.latest_run(root, 'compare') and os.path.basename(run.latest_run(root, 'compare')))
    if not name:
        return 0
    results = {}
    for n in [name] + args.extra_compare:
        for r in run.read_results(os.path.join(root, n)):
            results[(r['id'], r['mode'])] = r
    print(f'Baseline run: `{name}`' + (f', with results of {", ".join("`" + x + "`" for x in args.extra_compare)}' if args.extra_compare else '') + '\n')
    rows, sums = [], collections.Counter()
    modes = ('last-pass', 'from-scratch')
    for s in sources:
        rs = [r for r in results.values() if r['source'] == s]
        if not rs:
            continue
        row = [f'`{s}`']
        for mode in modes:
            here = [r for r in rs if r['mode'] == mode]
            comp = [r for r in here if r['outcome'] not in run.NOT_COMPARABLE]
            ident = sum(r['outcome'] == 'identical' for r in comp)
            row.append(f'{ident} / {len(comp)}')
            sums[mode + 'i'] += ident
            sums[mode + 'c'] += len(comp)
        rows.append(row)
    rows.append(['**total**'] + [f"**{sums[m + 'i']} / {sums[m + 'c']}**" for m in modes])
    table(['Source', 'Last pass', 'From scratch'], rows)
    outcomes = collections.Counter((r['mode'], r['outcome']) for r in results.values())
    for mode in modes:
        print(f'{mode}: ' + ', '.join(f'{k[1]} {v}' for k, v in sorted(outcomes.items()) if k[0] == mode))
    print()
    lp = [r for r in results.values() if r['mode'] == 'last-pass']
    level = collections.Counter()
    for r in lp:
        for lv in ('L1', 'L2', 'L6'):
            st = (r.get('levels', {}).get(lv) or {}).get('status')
            if st:
                level[(lv, st)] += 1
    print('Instrumented levels, last-pass results: ' + ', '.join(f'{k[0]} {k[1]} {v}' for k, v in sorted(level.items())))
    print()
    notes = [r for r in lp if r.get('inst_note')]
    print(f'Entries an instrumented level could not judge: {len(notes)}')
    for r in sorted(notes, key=lambda x: x['id']):
        if r['source'] in PRIVATE:
            continue
        print(f'- `{r["id"]}`: {r["inst_note"][:120]}')
    priv = sum(r['source'] in PRIVATE for r in notes)
    if priv:
        print(f'- and {priv} of the owner\'s documents')
    print()
    bad = sorted((r for r in results.values() if r['outcome'] not in run.NOT_COMPARABLE and r['outcome'] != 'identical'), key=lambda r: (r['id'], r['mode']))
    print(f'Comparisons not identical: {len(bad)}')
    for r in bad:
        if r['source'] in PRIVATE:
            print(f'- a document of the owner\'s [{r["mode"]}]: {r["outcome"]}')
        else:
            print(f'- `{r["id"]}` [{r["mode"]}]: {r["outcome"]}: {((r.get("first_divergence") or {}).get("text") or "")[:160]}')
    print()
    per_tier = collections.defaultdict(lambda: collections.Counter())
    for r in results.values():
        per_tier[r['tier']][r['mode'] + '-ms'] += r.get('candidate_ms') or 0
    print('Candidate wall time summed over entries (no instrumented pass, no comparison): ' + '; '.join(f'{t}: last pass {fmt_s(c["last-pass-ms"])}, from scratch {fmt_s(c["from-scratch-ms"])}' for t, c in per_tier.items()))
    slow = sorted((r for r in results.values() if r.get('candidate_ms')), key=lambda r: -r['candidate_ms'])[:5]
    print('Longest single candidate times: ' + ', '.join(f'{"(private)" if r["source"] in PRIVATE else r["id"]} {r["mode"]} {r["candidate_ms"] / 1000:.0f} s' for r in slow))
    return 0


if __name__ == '__main__':
    sys.exit(main())
