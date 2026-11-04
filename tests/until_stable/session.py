#!/usr/bin/env -S python3 -I
"""session.py: how often a real edit session asks for bibtex, biber or makeindex under `rstex --until-stable`, against how often latexmk runs one on the same edits.

    session.py --engine PATH [--ids REGEX] [--sample N] [--bib-sample N] [--biber-sample N] [--index-sample N] [--seed S] [--edits N] [--budget S] [--no-latexmk] [-j N] [--json FILE]

A document is mirrored under target/ as tools/prof/loop.py mirrors it, built to where it settles, and then gets --edits edits, a save for each, in random places at blank lines of its main file's body.  Most are what
typing is, and the rest what a session has now and then.  Each kind is reported apart:

    sentence    a new sentence and paragraph
    remove      an earlier edit taken out again
    label       a paragraph with a new \\label and a \\ref to it
    cite        a \\cite of a key the .aux already holds
    index       an \\index of a new word, if the document writes an index

An edit that makes a pass fail is taken back and counted.  When rstex says a program must run, it runs, and the build goes on, as a caller would do it; that is what the count is of.  latexmk gets the same edit in its own
output directory, under the same name in turn, and its runs of each program are counted from what it says.  After each save the two directories are held against each other, byte for byte, as compare.py does.  Only ids, kinds,
reasons, counts and the extensions of files that differ are printed, and text an edit adds is this file's own.
"""
import argparse
import collections
import concurrent.futures as cf
import json
import os
import random
import re
import shlex
import shutil
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))), 'tools', 'prof'))
import common  # noqa: E402
from common import proflib  # noqa: E402
import loop  # noqa: E402

RULE_RUNS = re.compile(rb"^Latexmk: applying rule '(\w+)", re.M)
KINDS = ('sentence', 'sentence', 'sentence', 'sentence', 'remove', 'label', 'cite', 'index')


class Builder:
    def __init__(self, doc, engine, src, main_rel, work):
        self.env = common.document_env(doc.entry, os.path.join(work, 'cache'))
        fmt = engine.format_path(doc.format)
        self.src = src
        cmd = f'{shlex.quote(engine.path)} -progname={doc.format} -fmt={shlex.quote(fmt)} %O %S'
        self.direct = [engine.path, '-progname=' + doc.format, '-fmt=' + fmt, '--until-stable'] + common.LEMMA_FLAGS + ['-output-directory=.build', './' + main_rel]
        self.latexmk = ['latexmk', '-norc', '-pdf', '-pdflatex=' + cmd] + common.LEMMA_FLAGS + ['-outdir=.build', './' + main_rel]
        self.job = doc.entry['jobname']
        self.out = os.path.join(src, '.build')
        self.other = os.path.join(src, '.build-latexmk')
        for out in (self.out, self.other):
            common.reset(out)
            common.mirror_dirs(src, out)

    def run_direct(self):
        """(status, passes, the lines that named a program).  The programs run and the build goes on."""
        lines, passes = [], 0
        for _ in range(6):
            r = subprocess.run(self.direct, cwd=self.src, env=self.env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, timeout=1800, check=False)
            done = common.PASSES_LINE.search(r.stderr)
            passes += int(done.group(1)) if done else 0
            if r.returncode != common.STATUS_TOOL:
                return r.returncode, passes, lines
            with open(os.devnull, 'wb') as null:
                for tool, reason, file in common.TOOL_LINE.findall(r.stderr):
                    lines.append((tool.decode(), reason.decode()))
                    common.run_tool(tool.decode(), file, os.path.join(self.src, '.build'), self.src, self.env, null)
        return r.returncode, passes, lines

    def run_latexmk(self):
        """In its own directory, under the name of the other: paths are in what it writes."""
        os.rename(self.out, self.out + '-direct')
        os.rename(self.other, self.out)
        try:
            r = subprocess.run(self.latexmk, cwd=self.src, env=self.env, stdin=subprocess.DEVNULL, capture_output=True, timeout=1800, check=False)
            self.got_latexmk = common.outputs(self.out)
        finally:
            os.rename(self.out, self.other)
            os.rename(self.out + '-direct', self.out)
        rules = [x.decode() for x in RULE_RUNS.findall(r.stdout + r.stderr)]
        return r.returncode, sum(1 for x in rules if x.endswith('latex')), [x for x in rules if not x.endswith('latex')]

    def differs(self):
        """What latexmk's last build has otherwise than this directory."""
        got = common.outputs(self.out)
        common.latexmk_only(self.got_latexmk, got, self.job)
        return common.differing(self.got_latexmk, got)

    def keys(self):
        """The keys the .aux cites, for an edit to cite one more time."""
        try:
            with open(os.path.join(self.src, '.build', self.job + '.aux'), 'rb') as f:
                return sorted({k for m in re.finditer(rb'^\\citation\{([^}]*)\}', f.read(), re.M) for k in m.group(1).split(b',') if k and k != b'*'})
        except OSError:
            return []


def edit_session(doc, engine, root, edits, seed, with_latexmk, budget):
    slug = re.sub(r'[^A-Za-z0-9_.-]', '_', doc.id)
    work = os.path.join(root, 'session', slug)
    src = os.path.join(work, 'src')
    main = common.mirror(doc.entry, src)
    b = Builder(doc, engine, src, os.path.relpath(main, src), work)
    res = {'id': doc.id, 'start': None, 'edits': collections.Counter(), 'fires': collections.Counter(), 'latexmk_runs': collections.Counter(), 'taken_back': 0, 'passes': 0, 'latexmk_passes': 0,
           'fires_by_kind': collections.Counter(), 'latexmk_by_kind': collections.Counter(), 'saves': 0, 'differing_saves': 0, 'differing_extensions': collections.Counter()}
    code, passes, lines = b.run_direct()
    res['start'] = {'status': code, 'passes': passes, 'tools': sorted({x[0] for x in lines})}
    if with_latexmk:
        res['start']['latexmk'] = b.run_latexmk()[0]
    if code != 0:
        res['skipped'] = 'does not build'
        shutil.rmtree(work, ignore_errors=True)
        return finish(res)
    rng = random.Random(seed)
    with open(main, 'rb') as f:
        text = f.read()
    added = []
    writes_index = os.path.exists(os.path.join(src, '.build', b.job + '.idx'))
    n = 0
    t0 = time.monotonic()
    while res['saves'] < edits and n < edits * 3:
        if time.monotonic() - t0 > budget:
            res['stopped'] = 'out of time'
            break
        n += 1
        kind = rng.choice(KINDS)
        at = body_places(text)
        if not at:
            res['skipped'] = 'no blank line in the body of the main file'
            break
        place = rng.choice(at)
        word = f'lemmaedit{n}'.encode()
        if kind == 'remove':
            if not added:
                continue
            piece = rng.choice(added)
            added.remove(piece)
            new = text.replace(piece, b'', 1)
        else:
            if kind == 'sentence':
                piece = b'These are ' + word + b' words of an edit.\n\n'
            elif kind == 'label':
                piece = b'See section \\ref{' + word + b'}\\label{' + word + b'}.\n\n'
            elif kind == 'cite':
                keys = b.keys()
                if not keys:
                    continue
                piece = b'As in \\cite{' + rng.choice(keys) + b'}, ' + word + b'.\n\n'
            else:
                if not writes_index:
                    continue
                piece = b'An index entry\\index{' + word + b'} is here.\n\n'
            new = text[:place] + piece + text[place:]
            added.append(piece)
        loop.save(main, new)
        code, passes, lines = b.run_direct()
        if code != 0:
            # What the edit broke is no matter here.  Back to where it was, before anything else is counted.
            res['taken_back'] += 1
            if kind != 'remove':
                added.remove(piece)
            loop.save(main, text)
            b.run_direct()
            if with_latexmk:
                b.run_latexmk()
            continue
        text = new
        res['saves'] += 1
        res['edits'][kind] += 1
        res['passes'] += passes
        for tool, reason in lines:
            res['fires'][f'{tool} {reason}'] += 1
            res['fires_by_kind'][f'{kind} {tool}'] += 1
        if with_latexmk:
            rc, lat_passes, runs = b.run_latexmk()
            res['latexmk_passes'] += lat_passes
            for tool in runs:
                res['latexmk_runs'][tool] += 1
                res['latexmk_by_kind'][f'{kind} {tool}'] += 1
            d = b.differs()
            if d:
                res['differing_saves'] += 1
                res['differing_extensions'].update(d)
    shutil.rmtree(work, ignore_errors=True)
    return finish(res)


def finish(res):
    for k, v in res.items():
        if isinstance(v, collections.Counter):
            res[k] = dict(v)
    return res


def body_places(text):
    begin = text.find(b'\\begin{document}')
    if begin < 0:
        return []
    end = text.find(b'\\end{document}', begin)
    end = len(text) if end < 0 else end
    return [begin + m.start() + 1 for m in re.finditer(rb'\n[ \t]*\n', text[begin:end])]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--engine', required=True)
    ap.add_argument('--ids')
    ap.add_argument('--sample', type=int, default=0)
    ap.add_argument('--bib-sample', type=int, default=0)
    ap.add_argument('--biber-sample', type=int, default=0)
    ap.add_argument('--index-sample', type=int, default=0)
    ap.add_argument('--seed', type=int, default=1)
    ap.add_argument('--edits', type=int, default=20)
    ap.add_argument('--budget', type=int, default=300, help='seconds for the edits of one document: a session stops there, and what it did counts')
    ap.add_argument('--no-latexmk', action='store_true')
    ap.add_argument('-j', type=int, default=4)
    ap.add_argument('--json')
    args = ap.parse_args()
    entries, _, _ = proflib.corpus().load_manifests(None)
    usable = [e for e in entries if e.get('main') and e['mode'] == 'latex' and os.path.isfile(e['main']) and not e.get('prelude') and not e.get('args')]
    picked = [e for e in usable if args.ids and re.search(args.ids, e['id'])]
    rng = random.Random(args.seed)
    public = [e for e in usable if e['id'].startswith(common.PUBLIC) and e not in picked]
    if args.sample:
        picked += rng.sample(public, min(args.sample, len(public)))
    for want, n in (('bibtex', args.bib_sample), ('biber', args.biber_sample), ('makeindex', args.index_sample)):
        if n:
            found = [e for e in public if e not in picked and common.looks_like(e, want)]
            picked += rng.sample(found, min(n, len(found)))
    docs = [proflib.Doc(e['id'], e) for e in picked]
    engine = proflib.Engine('engine', args.engine)
    for d in {d.format for d in docs}:
        engine.format_path(d)
    root = common.out_root()
    results = []
    with cf.ThreadPoolExecutor(args.j) as pool:
        futures = [pool.submit(edit_session, d, engine, root, args.edits, args.seed, not args.no_latexmk, args.budget) for d in docs]
        for f in cf.as_completed(futures):
            r = f.result()
            results.append(r)
            print(json.dumps(r, sort_keys=True), flush=True)
    results.sort(key=lambda r: r['id'])
    if args.json:
        with open(args.json, 'w') as f:
            json.dump(results, f, indent=1, sort_keys=True)
    # The sums, with the documents that took part.
    done = [r for r in results if r['saves']]
    saves = sum(r['saves'] for r in done)
    kinds = collections.Counter()
    fires = collections.Counter()
    fires_kind = collections.Counter()
    lat_kind = collections.Counter()
    for r in done:
        kinds.update(r['edits'])
        fires.update(r['fires'])
        fires_kind.update(r['fires_by_kind'])
        lat_kind.update(r['latexmk_by_kind'])
    print(f'{len(done)} of {len(results)} documents had a session; {saves} saves, {sum(r["passes"] for r in done)} passes direct, {sum(r["latexmk_passes"] for r in done)} under latexmk')
    print('edits:', dict(kinds))
    print('rstex named:', dict(fires))
    print('rstex by the kind of edit:', dict(fires_kind))
    print('latexmk ran, by the kind of edit:', dict(lat_kind))
    print(f'saves whose outputs differ from latexmk\'s: {sum(r["differing_saves"] for r in done)} of {saves}')


if __name__ == '__main__':
    main()
