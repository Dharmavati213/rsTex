#!/usr/bin/env -S python3 -I
"""compare.py: `rstex --until-stable` against `latexmk -pdflatex=<rstex>`, from the same start, on real documents.

    compare.py --engine PATH [--ids REGEX] [--sample N] [--seed S] [--bib-sample N] [--biber-sample N] [--index-sample N] [-j N] [--timeout S] [--warm] [--no-synctex] [--json FILE]

With no --ids the ladder comes first.  --sample adds that many public documents at random, and the others that many more, whose source says that they use the program, none of them of the hold-out or the reserve of tests/bench/README.md.

Each document runs in a new output directory under target/until-stable/, at one path for both sides, from a mirror of its tree there (tests/until_stable/common.py), with Lemma's flags and environment and a pinned clock:

    latexmk     latexmk -norc -pdf -pdflatex='<engine> -progname=pdflatex -fmt=<format> %O %S' <Lemma's flags> -outdir=<o> ./<main>
    direct      <engine> -progname=pdflatex -fmt=<format> --until-stable <Lemma's flags> -output-directory=<o> ./<main>, and when that says a program must run, that program, and again

Every file latexmk leaves there has to be the same, byte for byte, the PDF and the log among them, but for each driver's own record and what bibtex, biber and makeindex say on the way.
--warm runs direct once more from the same start with the checkpoints the last run left, so that restored passes are judged too, and then again from what that run left, with the PDF in its place, as a build with nothing to do.
--no-synctex leaves -synctex=1 out on both sides.  A document is named by its manifest id and a difference by
the extension of the file, so that nothing of a private document is printed.  The last line says PASS or FAIL; a document the two fail on alike counts as equal, and is said apart.
"""
import argparse
import concurrent.futures as cf
import json
import os
import random
import re
import shlex
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common  # noqa: E402
from common import proflib  # noqa: E402

RUN_NUMBER = re.compile(rb"^Run number \d+ of rule '(?:pdf)?latex", re.M)
RULE_RUNS = re.compile(rb"^Latexmk: applying rule '(\w+)", re.M)


def run_latexmk(doc, engine, src, main_rel, out, env, timeout):
    fmt = engine.format_path(doc.format)
    cmd = f'{shlex.quote(engine.path)} -progname={doc.format} -fmt={shlex.quote(fmt)} %O %S'
    argv = ['latexmk', '-norc', '-pdf', '-pdflatex=' + cmd] + common.LEMMA_FLAGS + ['-outdir=' + out, './' + main_rel]
    r = common.run_bounded(argv, src, env, timeout)
    text = r.stdout + r.stderr
    rules = RULE_RUNS.findall(text)
    return {'rc': r.returncode, 'passes': sum(1 for x in rules if x.endswith(b'latex')), 'tools': sorted(set(x.decode() for x in rules if not x.endswith(b'latex')))}


def run_direct(doc, engine, src, main_rel, out, env, timeout):
    fmt = engine.format_path(doc.format)
    argv = [engine.path, '-progname=' + doc.format, '-fmt=' + fmt, '--until-stable'] + common.LEMMA_FLAGS + ['-output-directory=' + out, './' + main_rel]
    passes, tools, invocations = 0, [], 0
    with open(os.devnull, 'wb') as null:
        while True:
            invocations += 1
            r = common.run_bounded(argv, src, env, timeout)
            done = common.PASSES_LINE.search(r.stderr)
            passes += int(done.group(1)) if done else 0
            if r.returncode != common.STATUS_TOOL or invocations > 8:
                return {'rc': r.returncode, 'passes': passes, 'tools': sorted(set(tools)), 'result': done.group(2).decode() if done else None}
            for tool, _, file in common.TOOL_LINE.findall(r.stderr):
                tools.append(tool.decode())
                if not common.run_tool(tool.decode(), file, out, src, env, null):
                    return {'rc': 'tool failed', 'passes': passes, 'tools': sorted(set(tools)), 'result': 'tool failed'}


def one(doc, engine, root, timeout, warm):
    slug = re.sub(r'[^A-Za-z0-9_.-]', '_', doc.id)
    work = os.path.join(root, 'cmp', slug)
    out = os.path.join(work, 'o')
    t0 = time.monotonic()
    res = {'id': doc.id}
    src = os.path.join(work, 'src')
    main_rel = os.path.relpath(common.mirror(doc.entry, src), src)
    try:
        common.reset(out)
        common.mirror_dirs(src, out)
        env = common.document_env(doc.entry, os.path.join(work, 'cache-a'))
        a = run_latexmk(doc, engine, src, main_rel, out, env, timeout)
        got_a = common.outputs(out)
        common.reset(out)
        common.mirror_dirs(src, out)
        env = common.document_env(doc.entry, os.path.join(work, 'cache-b'))
        b = run_direct(doc, engine, src, main_rel, out, env, timeout)
        got_b = common.outputs(out)
        res['unread_ind'] = common.latexmk_only(got_a, got_b, doc.entry['jobname'])
        res.update(latexmk=a, direct=b, differs=common.differing(got_a, got_b))
        if warm:
            common.reset(out)
            common.mirror_dirs(src, out)
            c = run_direct(doc, engine, src, main_rel, out, env, timeout)
            got_c = common.outputs(out)
            res.update(warm=c, warm_differs=common.differing(got_a, got_c))
            if c['rc'] == 0:
                d = run_direct(doc, engine, src, main_rel, out, env, timeout)
                res.update(again=d, again_differs=common.differing(got_a, common.outputs(out)))
        res['same_status'] = (a['rc'] == 0) == (b['rc'] == 0)
    except subprocess.TimeoutExpired:
        res['timeout'] = True
    res['seconds'] = round(time.monotonic() - t0, 1)
    subprocess.run(['rm', '-rf', work], check=False)
    return res


def choose(args):
    entries, _, _ = proflib.corpus().load_manifests(None)
    usable = [e for e in entries if e.get('main') and e['mode'] in ('latex', 'plain') and os.path.isfile(e['main']) and not e.get('prelude') and not e.get('args')]
    if args.ids:
        rx = re.compile(args.ids)
        picked = [e for e in usable if rx.search(e['id'])]
    else:
        picked = [e for e in usable if e['id'].startswith('ladder/')]
    rng = random.Random(args.seed)
    apart = common.kept_apart()
    public = [e for e in usable if e['id'].startswith(common.PUBLIC) and e not in picked and e['id'] not in apart]
    if args.sample:
        picked += rng.sample(public, min(args.sample, len(public)))
    rest = [e for e in public if e not in picked]
    for want, n in (('bibtex', args.bib_sample), ('biber', args.biber_sample), ('makeindex', args.index_sample)):
        if n:
            found = [e for e in rest if common.looks_like(e, want)]
            picked += rng.sample(found, min(n, len(found)))
    return [proflib.Doc(e['id'], e) for e in picked]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--engine', required=True)
    ap.add_argument('--ids')
    ap.add_argument('--sample', type=int, default=0)
    ap.add_argument('--bib-sample', type=int, default=0)
    ap.add_argument('--biber-sample', type=int, default=0)
    ap.add_argument('--index-sample', type=int, default=0)
    ap.add_argument('--seed', type=int, default=1)
    ap.add_argument('-j', type=int, default=4)
    ap.add_argument('--timeout', type=int, default=900)
    ap.add_argument('--warm', action='store_true')
    ap.add_argument('--no-synctex', action='store_true')
    ap.add_argument('--json')
    args = ap.parse_args()
    if args.no_synctex:
        common.LEMMA_FLAGS.remove('-synctex=1')
    root = common.out_root()
    engine = proflib.Engine('engine', args.engine)
    docs = choose(args)
    for d in {d.format for d in docs}:
        engine.format_path(d)
    results = []
    with cf.ThreadPoolExecutor(args.j) as pool:
        futures = {pool.submit(one, d, engine, root, args.timeout, args.warm): d for d in docs}
        for f in cf.as_completed(futures):
            r = f.result()
            results.append(r)
            bad = r.get('timeout') or r.get('differs') or r.get('warm_differs') or r.get('again_differs') or not r.get('same_status')
            print(json.dumps(r, sort_keys=True) if bad else f"same {r['id']}: passes {r['latexmk']['passes']} {r['direct']['passes']}, tools {r['direct']['tools']}, status {r['latexmk']['rc']} {r['direct']['rc']}, {r['seconds']} s", flush=True)
    results.sort(key=lambda r: r['id'])
    if args.json:
        with open(args.json, 'w') as f:
            json.dump(results, f, indent=1, sort_keys=True)
    failed = [r for r in results if r.get('timeout') or r.get('differs') or r.get('warm_differs') or r.get('again_differs') or not r.get('same_status')]
    both_fail = sum(1 for r in results if r not in failed and r['latexmk']['rc'] != 0)
    fewer = sum(1 for r in results if r not in failed and r['direct']['passes'] < r['latexmk']['passes'])
    more = sum(1 for r in results if r not in failed and r['direct']['passes'] > r['latexmk']['passes'])
    unread = sum(1 for r in results if r.get('unread_ind'))
    print(f'{len(results) - len(failed)}/{len(results)} equal, of which {both_fail} fail alike; direct took fewer passes on {fewer} and more on {more}; latexmk made a .ind that no pass read on {unread}')
    for r in failed:
        print('differs:', r['id'], r.get('differs'), r.get('warm_differs'), r.get('again_differs'), 'timeout' if r.get('timeout') else '')
    print('FAIL' if failed else 'PASS')
    sys.exit(1 if failed else 0)


if __name__ == '__main__':
    main()
