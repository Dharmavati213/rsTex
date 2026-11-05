#!/usr/bin/env -S python3 -I
"""kill.py: what a killed `rstex --until-stable` leaves, as an editor does it at each newer save.

    kill.py --engine PATH [--kills 40] [--sections 900] [--seed 1] [--mode lemma|chain|bare]... [--from-nothing 5] [--doc ID] [--check] [--json FILE]

A document written here, neutral, of --sections sections with a table of contents, a list of figures, hyperref's bookmarks, a \\include in a subdirectory and references across all of it, or with --doc a public document of the corpus manifests in a mirror of its tree, is built until it is stable.  Then, for each kill:

    an edit, and a build killed with SIGKILL at a random moment of it
    a look at what that left: the PDF, and the side files against what they held
    a newer edit, and a build, which is the run under test
    the same source and the files as they stood before the kill, built with no checkpoint, chain or output cache and no kill: the clean run

Every byte of what the two builds leave has to be the same, the PDF, the log, the `.fls`, `.synctex.gz` and each side file, and nothing of the kill may stay behind.  Every --from-nothing-th kill the clean run also starts from no file at all.

The modes are the flags: `lemma` is what Lemma passes, so a checkpoint and the output cache and no chain, since SyncTeX stops a chain; `chain` leaves out `-synctex`, so a pass resumes from page records; `bare` has no output directory and writes beside the source.

Before all that, --early kills in the first moments of a cold pass, after which a whole build must leave nothing that was not the job's.  Counts go to stdout and to --json: kills that landed, next builds that ended in a TeX error, kills that left the PDF empty, gone or cut, or left it the PDF of the build's earlier pass, which is a whole one, or left a side file not what it was, next builds that took nothing from the last PDF, and those that differ from the clean run.  With --check the last line says PASS or FAIL, on all of those being 0 but the two that are no fault: a whole PDF of an earlier pass, and the side files, which a pass writes under their own names until the next run has put them back.
"""
import argparse
import json
import os
import random
import re
import shutil
import signal
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common  # noqa: E402
from common import proflib  # noqa: E402

FIGURE = r"""\begin{figure}[h]\centering\rule{2cm}{1cm}\caption{A box of part %d}\end{figure}
"""
MODES = {
    'lemma': {'flags': common.LEMMA_FLAGS, 'out': 'o'},
    'chain': {'flags': ['-interaction=nonstopmode', '-file-line-error', '-recorder'], 'out': 'o'},
    'bare': {'flags': ['-interaction=nonstopmode', '-recorder'], 'out': None},
}
# What rstex itself leaves beside the outputs, and what no comparison counts.
KEPT_NAMES = ('.rstex-tmp', '.rstex-fls', '.synctex(busy)')


def document(sections, marks):
    """The main file.  `marks` maps a section to the sentences an edit has put there."""
    parts = []
    for k in range(sections):
        far = (k * 37 + 11) % sections
        parts.append(f'\\section{{Part {k}}}\\label{{p{k}}}\nThe words of part {k}, with a reference to part \\ref{{p{far}}} on page \\pageref{{p{far}}}.\n')
        if k % 40 == 7:
            parts.append(FIGURE % k)
        parts.extend(s + '\n' for s in marks.get(k, ()))
        if k == sections // 2:
            parts.append('\\include{sub/ch}\n')
    return ('\\documentclass{article}\n\\usepackage{hyperref}\n\\begin{document}\n\\tableofcontents\n\\listoffigures\n\\clearpage\n' + ''.join(parts) + '\\end{document}\n')


class DocumentTrial:
    """The same on a document of the corpus: the edits go at blank lines of its main file's body, and all it writes goes to a directory of its own, as for Lemma."""

    def __init__(self, engine, mode, doc, root):
        self.spec = MODES[mode]
        self.job = doc.entry['jobname']
        base = os.path.join(root, mode)
        common.reset(base)
        self.dir = os.path.join(base, 'src')
        self.main = common.mirror(doc.entry, self.dir)
        with open(self.main, 'rb') as f:
            self.text = f.read()
        self.env = common.document_env(doc.entry, os.path.join(base, 'cache'))
        self.env.pop('RSTEX_CHECKPOINT_POLICY', None)
        self.out = os.path.join(self.dir, 'o')
        common.reset(self.out)
        common.mirror_dirs(self.dir, self.out)
        rel = os.path.relpath(self.main, self.dir)
        self.argv = [engine.path, '-progname=' + doc.format, '-fmt=' + engine.format_path(doc.format), '--until-stable'] + self.spec['flags'] + ['-output-directory=o', './' + rel]

    def edit(self, rng, n, with_section):
        begin = self.text.find(b'\\begin{document}')
        end = self.text.find(b'\\end{document}', begin)
        places = [begin + m.start() + 1 for m in re.finditer(rb'\n[ \t]*\n', self.text[begin:end])]
        at = rng.choice(places)
        piece = (f'\\section*{{Added {n}}}\n' if with_section else '').encode() + f'These are the words of an edit, number {n}.\n\n'.encode()
        self.text = self.text[:at] + piece + self.text[at:]
        with open(self.main, 'wb') as f:
            f.write(self.text)


class Trial:
    def __init__(self, engine, mode, sections, root):
        self.spec = MODES[mode]
        self.job = 'a'
        self.dir = os.path.join(root, mode)
        common.reset(self.dir)
        self.sections = sections
        self.marks = {}
        self.env = dict(os.environ)
        self.env.update(common.LEMMA_ENV)
        self.env.update(proflib.corpus().DETERMINISM)
        self.env.update(proflib.corpus().HERMETIC)
        self.env['XDG_CACHE_HOME'] = os.path.join(self.dir, 'cache')
        self.env.pop('RSTEX_CHECKPOINT_POLICY', None)
        self.engine = engine
        out = self.spec['out']
        self.argv = [engine.path, '-progname=pdflatex', '-fmt=' + engine.format_path('pdflatex'), '--until-stable'] + self.spec['flags'] + (['-output-directory=' + out] if out else []) + ['./a.tex']
        self.out = os.path.join(self.dir, out) if out else self.dir
        os.makedirs(self.out, exist_ok=True)
        os.makedirs(os.path.join(self.out, 'sub'), exist_ok=True)
        os.makedirs(os.path.join(self.dir, 'sub'), exist_ok=True)
        with open(os.path.join(self.dir, 'sub', 'ch.tex'), 'w', encoding='utf-8') as f:
            f.write('\\section{Included}\\label{included}\nThe words of a chapter on its own, with a reference to part \\ref{p3}.\n')
        self.save()

    def save(self):
        with open(os.path.join(self.dir, 'a.tex'), 'w', encoding='utf-8') as f:
            f.write(document(self.sections, self.marks))

    def edit(self, rng, n, with_section):
        k = rng.randrange(self.sections)
        if with_section:
            self.marks.setdefault(k, []).append(f'\\section{{Added {n}}}\\label{{added{n}}}\nThe words of an added part, number {n}.')
        else:
            self.marks.setdefault(k, []).append(f'The words of an edit, number {n}.')
        self.save()

    def build(self, env=None):
        return subprocess.run(self.argv, cwd=self.dir, env=env or self.env, stdin=subprocess.DEVNULL, capture_output=True, timeout=1800, check=False)

    def files(self):
        """Relative path -> bytes of everything the engine leaves, in the output directory, this file's own sources aside, and the names of what a kill left."""
        res, left = {}, []
        for root, dirs, names in os.walk(self.out):
            dirs[:] = [d for d in dirs if d not in ('cache', 'o-kept', 'o-clean')]
            for f in names:
                rel = os.path.relpath(os.path.join(root, f), self.out)
                if rel in ('a.tex', os.path.join('sub', 'ch.tex')):
                    continue
                if rel.endswith(KEPT_NAMES):
                    left.append(os.path.splitext(rel)[1])
                    continue
                if rel.endswith(common.NOT_COMPARED):
                    continue
                with open(os.path.join(root, f), 'rb') as fh:
                    res[rel] = fh.read()
        return res, left

    def put_files(self, files):
        """The output directory as `files` has it, and with nothing else in it."""
        have, _ = self.files()
        for root, _, names in os.walk(self.out):
            for f in names:
                rel = os.path.relpath(os.path.join(root, f), self.out)
                if rel not in ('a.tex', os.path.join('sub', 'ch.tex')) and (rel in have or rel.endswith(KEPT_NAMES) or rel.endswith(common.NOT_COMPARED)):
                    os.remove(os.path.join(root, f))
        for rel, data in files.items():
            with open(os.path.join(self.out, rel), 'wb') as fh:
                fh.write(data)

    def killed_build(self, at):
        """A build, killed `at` seconds in.  False if it ended first."""
        p = subprocess.Popen(self.argv, cwd=self.dir, env=self.env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
        time.sleep(at)
        if p.poll() is not None:
            return False
        os.killpg(p.pid, signal.SIGKILL)
        p.wait()
        return p.returncode == -signal.SIGKILL


for name in ('build', 'files', 'put_files', 'killed_build'):
    setattr(DocumentTrial, name, getattr(Trial, name))


def differs(a, b):
    return {os.path.splitext(k)[1] or k: 1 for k in set(a) | set(b) if a.get(k) != b.get(k)}


def keep_difference(args, mode, n, got, other, before):
    """With --keep DIR, under target/: what the two builds left, for reading the difference.  The document is this file's own, or a public one."""
    if not args.keep:
        return
    for name, files in (('next', got), ('clean', other), ('before', before)):
        for rel, data in files.items():
            p = os.path.join(common.under_target_path(args.keep), f'{mode}-{n}', name, rel)
            os.makedirs(os.path.dirname(p), exist_ok=True)
            with open(p, 'wb') as f:
                f.write(data)


def early_kills(t, rng, kills):
    """Kills in a cold pass's first moments, and then a whole build: how many names it leaves that are not the job's, for good.  Files of a process that died have no one else to take them away."""
    env = dict(t.env, RSTEX_NO_SNAPSHOT='1')
    for _ in range(kills):
        p = subprocess.Popen(t.argv, cwd=t.dir, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
        time.sleep(rng.uniform(0, 0.2))
        if p.poll() is None:
            os.killpg(p.pid, signal.SIGKILL)
        p.wait()
    t.build()
    others = [n for n in os.listdir(t.out) if os.path.isfile(os.path.join(t.out, n)) and not n.startswith(t.job + '.') and n.endswith('.fls')]
    for n in others:
        os.remove(os.path.join(t.out, n))
    return len(others)


def run_mode(engine, mode, args, root, rng):
    doc = proflib.resolve_docs([args.doc])[0] if args.doc else None
    t = DocumentTrial(engine, mode, doc, root) if doc else Trial(engine, mode, args.sections, root)
    clean_env = dict(t.env, RSTEX_NO_SNAPSHOT='1')
    trace_env = dict(t.env, RSTEX_OUTCACHE_TRACE='1', RSTEX_CHECKPOINT_TRACE='1')
    for _ in range(3):
        r = t.build()
        if r.returncode != 0:
            raise SystemExit(f'{mode}: the document does not build: status {r.returncode} {r.stderr[-300:]!r}')
    # How long a build takes that passes twice, which is what an edit that moves things asks for.
    t.edit(rng, 0, True)
    t0 = time.monotonic()
    t.build()
    full = time.monotonic() - t0
    res = {'mode': mode, 'early_left': early_kills(t, rng, args.early), 'kills': 0, 'ended_first': 0, 'pdf_empty_or_gone': 0, 'pdf_cut': 0, 'pdf_of_an_earlier_pass': 0, 'side_not_as_was': 0, 'next_error': 0, 'next_status': {}, 'no_hits': 0, 'differ': 0, 'differ_ext': {}, 'differ_nothing': 0, 'left_behind': 0, 'full_build_s': round(full, 3), 'passes': 0, 'clean_passes': 0}
    n = 0
    tries = 0
    while res['kills'] < args.kills and tries < args.kills * 4:
        tries += 1
        n += 1
        before, _ = t.files()
        t.edit(rng, n, rng.random() < 0.25)
        if not t.killed_build(rng.uniform(0, full)):
            res['ended_first'] += 1
            continue
        res['kills'] += 1
        after, _ = t.files()
        pdf = after.get(t.job + '.pdf')
        if not pdf:
            res['pdf_empty_or_gone'] += 1
        elif not (pdf.startswith(b'%PDF-') and pdf.rstrip().endswith(b'%%EOF')):
            res['pdf_cut'] += 1
        elif pdf != before.get(t.job + '.pdf'):
            res['pdf_of_an_earlier_pass'] += 1
        if any(after.get(k) != before.get(k) for k in before if k.endswith(('.aux', '.toc', '.lof', '.out'))):
            res['side_not_as_was'] += 1
        # The newer save, and the run under test.
        t.edit(rng, f'{n}b', False)
        r = subprocess.run(t.argv, cwd=t.dir, env=trace_env, stdin=subprocess.DEVNULL, capture_output=True, timeout=1800, check=False)
        status = str(r.returncode)
        res['next_status'][status] = res['next_status'].get(status, 0) + 1
        if r.returncode != 0:
            res['next_error'] += 1
        m = common.PASSES_LINE.search(r.stderr)
        res['passes'] += int(m.group(1)) if m else 0
        # A first pass with no stream from the last PDF has lost the output cache.
        first = re.search(rb'outcache: streams (\d+) hits (\d+)', r.stderr)
        if not first or first.group(2) == b'0':
            res['no_hits'] += 1
        got, left = t.files()
        if left:
            res['left_behind'] += 1
        # The clean run: from the files as they were before the kill.
        t.put_files(before)
        r2 = t.build(clean_env)
        m = common.PASSES_LINE.search(r2.stderr)
        res['clean_passes'] += int(m.group(1)) if m else 0
        clean, _ = t.files()
        d = differs(got, clean)
        if d:
            res['differ'] += 1
            for k in d:
                res['differ_ext'][k] = res['differ_ext'].get(k, 0) + 1
            keep_difference(args, mode, res['kills'], got, clean, before)
        if args.from_nothing and res['kills'] % args.from_nothing == 0:
            t.put_files({})
            os.makedirs(os.path.join(t.out, 'sub'), exist_ok=True)
            r3 = t.build(clean_env)
            nothing, _ = t.files()
            if r3.returncode != r.returncode or differs(got, nothing):
                res['differ_nothing'] += 1
                keep_difference(args, mode, f'{res["kills"]}-nothing', got, nothing, before)
        # On from what the build under test left, with its cache as it left it.
        t.put_files(got)
    return res


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--engine', required=True)
    ap.add_argument('--kills', type=int, default=40)
    ap.add_argument('--sections', type=int, default=900)
    ap.add_argument('--seed', type=int, default=1)
    ap.add_argument('--mode', action='append', choices=sorted(MODES))
    ap.add_argument('--from-nothing', type=int, default=5)
    ap.add_argument('--early', type=int, default=40, help='kills in the first moments of a cold pass, before the rest')
    ap.add_argument('--doc', help='a public document: its id, by tools/prof/proflib.py')
    ap.add_argument('--check', action='store_true')
    ap.add_argument('--work', default='kill')
    ap.add_argument('--keep', help='a directory under target/ for the files of a build that differs')
    ap.add_argument('--json')
    args = ap.parse_args()
    engine = proflib.Engine('engine', args.engine)
    if args.doc and not args.doc.startswith(common.PUBLIC):
        raise SystemExit('--doc takes a public document')
    root = os.path.join(common.out_root(), args.work)
    results = []
    bad = 0
    for mode in args.mode or ['lemma', 'chain', 'bare']:
        res = run_mode(engine, mode, args, root, random.Random(args.seed))
        results.append(res)
        print(json.dumps(res, sort_keys=True), flush=True)
        bad += res['next_error'] + res['pdf_empty_or_gone'] + res['pdf_cut'] + res['no_hits'] + res['differ'] + res['differ_nothing'] + res['left_behind'] + res['early_left']
        if res['kills'] < args.kills:
            bad += 1
    if args.json:
        with open(args.json, 'w') as f:
            json.dump(results, f, indent=1, sort_keys=True)
    shutil.rmtree(root, ignore_errors=True)
    if args.check:
        print('FAIL' if bad else 'PASS')
        sys.exit(1 if bad else 0)


if __name__ == '__main__':
    main()
