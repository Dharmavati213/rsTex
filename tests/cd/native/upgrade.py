#!/usr/bin/env -S python3 -I
r"""What a tree's update does to what the cache holds: a tree with one macro changed falls back through a checkpoint and through page records, with nothing stale, and gives what a cold run with the path off gives.  DESIGN section 10, "Versions".

    python3 -I tests/cd/native/upgrade.py --engine BIN [--work DIR]

BIN is a plain build called rstex.  The tree is the one whose bin directory PATH holds.  It is copied to --work, reflinked where the file system can, and only the copy is changed; PATH then names the copy, and the one document here is made up
on the spot, with nothing of anyone's in it.  Every run has the same cache, as an editor's would.

  pristine   cold, restored, resumed from a record after an edit near the end: every diagram on the path, the font path taken.
  pgf        one macro of PGF's changed in its file, \pgfscope, nothing else: the same cache.  No restore and no resume, by what the trace says, every diagram declined and naming that macro, and log and PDF the bytes of a cold run with the path off from
             an empty cache.  Then restored and resumed on what this tree wrote: the same again.
  back       the file as it was, with a new stamp: what was written for the changed file does not hold, the path is back, and the bytes are those of the pristine run.
  latex      one macro of LaTeX's changed in latex.ltx, \leavevmode: a new format with no word about it, all declined, the same bytes as with the path off.
  microtype  one macro of the font path's changed in microtype.sty: nothing armed there, the path for diagrams as it was, the same bytes as with RSTEX_NO_PF.
  back again the tree as it was: all of it armed, and the bytes of the pristine run.

Exit status 0 if all of it holds.  Prints what it found, by counts.
"""
import argparse
import os
import re
import shutil
import subprocess
import sys

ENV = {'SOURCE_DATE_EPOCH': '1700000000', 'FORCE_SOURCE_DATE': '1', 'TZ': 'UTC', 'LC_ALL': 'C', 'LANG': 'C', 'HOME': os.environ.get('HOME', '/')}
FATE = re.compile(rb'^\[\[commute (native|fallback|failure|restart)\]\] ?(.*)$', re.M)
PARAGRAPH = 'This is text of no importance, long enough to fill a line or two of the page, with a word here and there, and then it stops and the next one begins as this one did. ' * 5
DIAGRAM = '\\begin{tikzcd} A \\arrow[r, "f"] \\arrow[d, "g"\'] & B \\arrow[d, "h"] \\\\ C \\arrow[r, "k"\'] & D \\end{tikzcd}'


def document(last_word='Done'):
    body = ''.join(f'\\section{{Part {k}}}\n{PARAGRAPH}\n\n{DIAGRAM}\n\n{PARAGRAPH}\n\n{PARAGRAPH}\n\n{PARAGRAPH}\n\n' for k in range(40))
    return '\\documentclass{article}\n\\usepackage{microtype}\n\\usepackage{tikz-cd}\n\\begin{document}\n' + body + last_word + '\n\\end{document}\n'


class Run:
    def __init__(self, r, out):
        self.status, self.trace = r.returncode, r.stderr.decode('latin-1')
        self.log = open(os.path.join(out, 'x.log'), 'rb').read()
        self.pdf = open(os.path.join(out, 'x.pdf'), 'rb').read()
        self.fates = [m.group(1).decode() for m in FATE.finditer(self.log)]
        self.reasons = [m.group(2).decode('latin-1').removesuffix('%').strip() for m in FATE.finditer(self.log)]

    @property
    def restored(self):
        return 'rstex: checkpoint: restored' in self.trace.split('\n')

    @property
    def resumed(self):
        m = re.search(r'pages: \d+ records, using (\d+)', self.trace)
        return bool(m and int(m.group(1)) > 0)

    @property
    def clean_log(self):
        """The log without the lines of --commute-report, and the one line that has the run's cache in it."""
        log = re.sub(rb'\n\[\[commute [^\n]* %\n', b'', self.log)
        return re.sub(rb'(?m)^\[\[commute [^\n]*\n', b'', log)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--engine', required=True)
    ap.add_argument('--work', default='target/native/upgrade')
    args = ap.parse_args()
    work = os.path.abspath(args.work)
    engine = os.path.abspath(args.engine)
    if os.path.basename(engine) != 'rstex':
        sys.exit('the binary has to be called rstex')
    shutil.rmtree(work, ignore_errors=True)
    os.makedirs(work)
    latex = shutil.which('pdflatex')
    if not latex:
        sys.exit('no pdflatex on PATH: it says which tree this is')
    bindir = os.path.dirname(os.path.realpath(latex))
    root = os.path.dirname(os.path.dirname(bindir))
    if not os.path.isdir(os.path.join(root, 'texmf-dist')):
        sys.exit(f'{root} has no texmf-dist: this copies a tree as TeX Live installs it, and a distribution\'s tree under /usr is not that')
    copy = os.path.join(work, 'tree')
    print(f'copying {root}', flush=True)
    subprocess.run(['cp', '-a', '--reflink=auto', root, copy], check=True)
    newbin = os.path.join(copy, os.path.relpath(bindir, root))
    path = os.pathsep.join(newbin if os.path.realpath(p) == bindir else p for p in os.environ['PATH'].split(os.pathsep))
    cache = os.path.join(work, 'cache')
    d = os.path.join(work, 'doc')
    os.makedirs(d)

    def tex(extra=(), env=None, cache_dir=cache, source=None):
        with open(os.path.join(d, 'x.tex'), 'w') as f:
            f.write(source or document())
        e = dict(ENV, PATH=path, XDG_CACHE_HOME=cache_dir, RSTEX_CHECKPOINT_POLICY='always', RSTEX_CHECKPOINT_TRACE='1', RSTEX_PAGE_CK='4', **(env or {}))
        r = subprocess.run([engine, '-progname=pdflatex', '-interaction=batchmode', '-no-shell-escape', *extra, 'x.tex'], cwd=d, env=e, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, timeout=600)
        return Run(r, d)

    fails = []

    def check(ok, what):
        print(f"  {'ok  ' if ok else 'FAIL'} {what}", flush=True)
        if not ok:
            fails.append(what)

    def cold(extra=(), env=None, source=None):
        """From an empty cache, with the checkpoint off."""
        scratch = os.path.join(work, 'cold-cache')
        shutil.rmtree(scratch, ignore_errors=True)
        return tex(extra=['--no-snapshot', *extra], env=env, cache_dir=scratch, source=source)

    def same(a, b, what):
        check(a.clean_log == b.clean_log, f'{what}: the same log')
        check(a.pdf == b.pdf, f'{what}: the same PDF')

    def edit(rel, old, new):
        p = os.path.join(copy, rel)
        # What a package manager does: another file in its place.
        with open(p, 'rb') as f:
            data = f.read()
        if old is not None:
            if data.count(old) < 1:
                sys.exit(f'{rel} has no {old!r}: this tree is not as this tool expects')
            changed = data.replace(old, new, 1)
        else:
            changed = new
        os.remove(p)
        with open(p, 'wb') as f:
            f.write(changed)
        return data

    print('pristine')
    a1 = tex(['--commute-report'])
    check(a1.fates == ['native'] * 40, f'cold: {a1.fates.count("native")} of 40 on the path')
    a2 = tex(['--commute-report'])
    check(a2.restored and a2.fates == ['native'] * 40, f'restored, {a2.fates.count("native")} on the path')
    same(a1, a2, 'restored against cold')
    edited = document('Finished')
    a3 = tex(['--commute-report'], source=edited)
    check(a3.resumed and a3.fates.count('native') < 40, f'an edit near the end resumes from a record, {a3.fates.count("native")} diagrams left to do')
    pf = tex(env={'RSTEX_PF_STATS': '1'}, cache_dir=os.path.join(work, 'pf-cache'))
    check(re.search(r'pf: .* taken [1-9]', pf.trace) is not None, 'the font path is taken')
    pristine = tex(['--commute-report'])

    print('pgf: one macro of PGF, \\pgfscope')
    scopes = 'texmf-dist/tex/generic/pgf/basiclayer/pgfcorescopes.code.tex'
    original = edit(scopes, b'\\def\\pgfscope{%\n', b'\\def\\pgfscope{\\relax%\n')
    b1 = tex(['--commute-report'])
    check(not b1.restored and not b1.resumed, 'the old checkpoint and chain are not used')
    check(b1.fates == ['fallback'] * 40, f'{b1.fates.count("fallback")} of 40 declined')
    check(all('not as installed: pgfscope' in r for r in b1.reasons), 'each says it is \\pgfscope' + (f', not {b1.reasons[0]}' if b1.reasons and 'not as installed: pgfscope' not in b1.reasons[0] else ''))
    off = cold(['--no-commute'])
    same(b1, off, 'against a cold run with the path off')
    b2 = tex(['--commute-report'])
    check(b2.restored and b2.fates == ['fallback'] * 40, 'restored from what this tree wrote, and all declined again')
    same(b2, off, 'restored against a cold run with the path off')
    b3 = tex(['--commute-report'], source=edited)
    off2 = cold(['--no-commute'], source=edited)
    check(b3.resumed and 'native' not in b3.fates, 'an edit resumes from a record of this tree, and takes no path')
    same(b3, off2, 'resumed against a cold run with the path off')

    print('back: the file as it was, under a new stamp')
    edit(scopes, None, original)
    c1 = tex(['--commute-report'])
    check(c1.fates == ['native'] * 40, f'{c1.fates.count("native")} of 40 on the path')
    same(c1, pristine, 'against the pristine run')

    print('latex: one macro of LaTeX, \\leavevmode')
    ltx = 'texmf-dist/tex/latex/base/latex.ltx'
    original = edit(ltx, b'\\def\\leavevmode{\\unhbox\\voidb@x}', b'\\def\\leavevmode{\\unhbox\\voidb@x\\relax}')
    d1 = tex(['--commute-report'])
    check(not d1.restored and not d1.resumed and d1.fates == ['fallback'] * 40, f'nothing restored, {d1.fates.count("fallback")} of 40 declined, with a format built without a word')
    check(all('leavevmode' in r for r in d1.reasons), 'each says it is \\leavevmode' + (f', not {d1.reasons[0]}' if d1.reasons and 'leavevmode' not in d1.reasons[0] else ''))
    off = cold(['--no-commute'])
    same(d1, off, 'against a cold run with the path off')
    edit(ltx, None, original)

    print('microtype: one macro of the font path, \\MT@split@name')
    sty = 'texmf-dist/tex/latex/microtype/microtype.sty'
    original = edit(sty, b'\\def\\MT@split@name#1/#2/#3/#4/#5/#6\\@nil{%\n', b'\\def\\MT@split@name#1/#2/#3/#4/#5/#6\\@nil{\\relax%\n')
    e1 = tex(['--commute-report'], env={'RSTEX_PF_STATS': '1'})
    check(not e1.restored and e1.fates == ['native'] * 40, f'nothing restored, and {e1.fates.count("native")} of 40 diagrams still on their path')
    check(re.search(r'pf: .* taken 0;', e1.trace) is not None, 'the font path is not taken')
    no_pf = cold(['--commute-report'], env={'RSTEX_NO_PF': '1'})
    same(e1, no_pf, 'against a run with the real macros')
    edit(sty, None, original)

    print('back again: the tree as it was')
    f1 = tex(['--commute-report'])
    check(f1.fates == ['native'] * 40, f'{f1.fates.count("native")} of 40 on the path')
    same(f1, pristine, 'against the pristine run')
    print(f'{len(fails)} failed' + (': ' + '; '.join(fails) if fails else ''))
    if not fails:
        shutil.rmtree(work, ignore_errors=True)
    return 1 if fails else 0


if __name__ == '__main__':
    sys.exit(main())
