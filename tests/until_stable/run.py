#!/usr/bin/env -S python3 -I
"""run.py: `rstex --until-stable` on small documents written here, with LaTeX and the real bibtex, biber and makeindex.  crates/rstex/tests/until_stable.rs has what needs no TeX tree, and compare.py real documents.

    run.py --engine PATH [--only NAME]...

What each case holds is in its docstring.  They run under target/until-stable/cases/.  At each step of a session the bytes are held against latexmk on the same source, and for the flags Lemma passes, against the oracle
behind latexmk, as far as the log's banner and memory figures go.  The last line says PASS or FAIL.
"""
import argparse
import os
import re
import shutil
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common  # noqa: E402
from common import proflib  # noqa: E402

REFS = r"""\documentclass{article}
\usepackage{hyperref}
\begin{document}
\tableofcontents
\section{One}\label{one}
See section \ref{two} on page \pageref{two}.
\newpage
\section{Two}\label{two}
And section \ref{one}.
\end{document}
"""
BIB = r"""@book{alpha, author={A. Author}, title={Alpha}, year={2001}, publisher={P}}
@book{beta, author={B. Author}, title={Beta}, year={2002}, publisher={P}}
@book{gamma, author={C. Author}, title={Gamma}, year={2003}, publisher={P}}
"""
BIBTEX = r"""\documentclass{article}
\begin{document}
\section{One}\label{one}
Words, and a citation \cite{alpha}.

MARK

\bibliographystyle{plain}
\bibliography{refs}
\end{document}
"""
BIBER = r"""\documentclass{article}
\usepackage{biblatex}
\addbibresource{refs.bib}
\begin{document}
Words, and a citation \cite{alpha}.

MARK

\printbibliography
\end{document}
"""
INDEX = r"""\documentclass{article}
\usepackage{makeidx}
\makeindex
\begin{document}
Words\index{word}, and more\index{more}.

MARK

\printindex
\end{document}
"""


class Case:
    def __init__(self, name, engine):
        self.name, self.engine = name, engine
        self.dir = os.path.join(common.out_root(), 'cases', name)
        shutil.rmtree(self.dir, ignore_errors=True)
        os.makedirs(self.dir)
        self.env = dict(os.environ)
        self.env.update(common.LEMMA_ENV)
        self.env.update(proflib.corpus().DETERMINISM)
        self.env.update(proflib.corpus().HERMETIC)
        self.env['XDG_CACHE_HOME'] = os.path.join(self.dir, 'cache')
        self.fmt = engine.format_path('pdflatex')
        self.flags = ['-progname=pdflatex', '-fmt=' + self.fmt]

    def write(self, name, text):
        p = os.path.join(self.dir, name)
        os.makedirs(os.path.dirname(p), exist_ok=True)
        with open(p, 'w', encoding='utf-8') as f:
            f.write(text)

    def direct(self, main='a.tex', out='o', extra=(), env=None, lemma=True):
        os.makedirs(os.path.join(self.dir, out or '.'), exist_ok=True)
        argv = [self.engine.path] + self.flags + ['--until-stable'] + (common.LEMMA_FLAGS if lemma else []) + list(extra) + (['-output-directory=' + out] if out else []) + ['./' + main]
        return subprocess.run(argv, cwd=self.dir, env=env or self.env, stdin=subprocess.DEVNULL, capture_output=True, timeout=600, check=False)

    def plain(self, main='a.tex', out='o', extra=(), env=None):
        os.makedirs(os.path.join(self.dir, out or '.'), exist_ok=True)
        argv = [self.engine.path] + self.flags + common.LEMMA_FLAGS + list(extra) + (['-output-directory=' + out] if out else []) + ['./' + main]
        return subprocess.run(argv, cwd=self.dir, env=env or self.env, stdin=subprocess.DEVNULL, capture_output=True, timeout=600, check=False)

    def latexmk(self, main='a.tex', out='o', tex=None):
        cmd = tex or f'{self.engine.path} ' + ' '.join(self.flags) + ' %O %S'
        os.makedirs(os.path.join(self.dir, out), exist_ok=True)
        argv = ['latexmk', '-norc', '-pdf', '-pdflatex=' + cmd] + common.LEMMA_FLAGS + ['-outdir=' + out, './' + main]
        return subprocess.run(argv, cwd=self.dir, env=self.env, stdin=subprocess.DEVNULL, capture_output=True, timeout=600, check=False)

    def follow(self, out='o', main='a.tex'):
        """A build as a caller makes it: `--until-stable`, and when a program is named, that, and again.  (the lines that named one, passes, status)"""
        named, passes = [], 0
        for _ in range(6):
            r = self.direct(main=main, out=out)
            m = common.PASSES_LINE.search(r.stderr)
            passes += int(m.group(1)) if m else 0
            if r.returncode != common.STATUS_TOOL:
                return named, passes, r.returncode
            with open(os.devnull, 'wb') as null:
                for tool, reason, file in common.TOOL_LINE.findall(r.stderr):
                    named.append((tool.decode(), reason.decode()))
                    ok = common.run_tool(tool.decode(), file, os.path.join(self.dir, out), self.dir, self.env, null)
                    check(ok, f'{tool} failed')
        return named, passes, r.returncode

    def outputs(self, out='o'):
        return common.outputs(os.path.join(self.dir, out))


class Failed(Exception):
    pass


def check(cond, what):
    if not cond:
        raise Failed(what)


def same(a, b, what):
    d = common.differing(a, b)
    check(not d, f'{what}: differ {d}')


def session(c, steps, tool):
    """Each step edits the source and builds both ways, each in the output directory of its own, under one name in turn so that paths in the outputs are the same."""
    a, b = os.path.join(c.dir, 'o'), os.path.join(c.dir, 'o-latexmk')
    os.makedirs(b)
    os.makedirs(a)
    for name, edit, want in steps:
        edit()
        # Stamps have to move, whichever way the two run.
        time.sleep(0.02)
        os.rename(a, a + '-direct')
        os.rename(b, a)
        r = c.latexmk()
        check(r.returncode == 0, f'{name}: latexmk says {r.returncode}')
        got_latexmk = c.outputs()
        os.rename(a, b)
        os.rename(a + '-direct', a)
        named, _, code = c.follow()
        check(code == 0, f'{name}: direct says {code}')
        check([x[1] for x in named] == want, f'{name}: named {named}, wanted {want}')
        check(all(x[0] == tool for x in named), f'{name}: named {named}')
        got = c.outputs()
        common.latexmk_only(got_latexmk, got, 'a')
        same(got_latexmk, got, name)


def case_refs_against_latexmk(engine):
    """A first build, then a change that moves a page, each to its fixed point.  The second pass is on the checkpoint the first left."""
    c = Case('refs', engine)
    c.write('a.tex', REFS)
    os.makedirs(os.path.join(c.dir, 'o'))
    os.makedirs(os.path.join(c.dir, 'o-l'))
    c.env['RSTEX_CHECKPOINT_POLICY'] = 'always'
    r = c.direct()
    check(r.returncode == 0, f'status {r.returncode}')
    check(b'passes=3 result=stable' in r.stderr or b'passes=2 result=stable' in r.stderr, r.stderr)
    first = c.outputs()
    shutil.rmtree(os.path.join(c.dir, 'o'))
    c.env['XDG_CACHE_HOME'] = os.path.join(c.dir, 'cache-l')
    check(c.latexmk().returncode == 0, 'latexmk')
    got = c.outputs()
    same(got, first, 'first build')
    # A checkpoint from a first build now restores in each pass of another, with the same result.
    shutil.rmtree(os.path.join(c.dir, 'o'))
    c.env['XDG_CACHE_HOME'] = os.path.join(c.dir, 'cache')
    c.env['RSTEX_CHECKPOINT_TRACE'] = '1'
    r = c.direct()
    check(r.stderr.count(b'rstex: checkpoint: restored') >= 2, r.stderr)
    same(got, c.outputs(), 'restored passes')


def case_lemma_flags_against_the_oracle(engine):
    """Everything Lemma passes, in a directory it has made, with a \\include in a subdirectory, a long line in the log and a warning: to the fixed point, against pdflatex behind latexmk."""
    c = Case('lemma-flags', engine)
    c.write('a.tex', r"""\documentclass{article}
\includeonly{sub/ch}
\begin{document}
\typeout{""" + 'a very long line ' * 20 + r"""}
\tableofcontents
\include{sub/ch}
\section{Back}\label{back}
On page \pageref{there}, and no such \ref{nowhere}.
\end{document}
""")
    c.write('sub/ch.tex', r"""\section{There}\label{there}
Page \pageref{back}.
""")
    os.makedirs(os.path.join(c.dir, 'o', 'sub'))
    r = c.direct()
    check(r.returncode == 0, f'status {r.returncode}')
    ours = c.outputs()
    log = ours['a.log']
    check(b'a very long line ' * 20 in log, 'max_print_line: the line is not whole in the log')
    check(os.path.isfile(os.path.join(c.dir, 'o', 'sub', 'ch.aux')) and 'a.fls' in ours and 'a.toc' in ours, 'the files')
    shutil.rmtree(os.path.join(c.dir, 'o'))
    os.makedirs(os.path.join(c.dir, 'o', 'sub'))
    check(c.latexmk(tex='pdflatex %O %S').returncode == 0, 'latexmk')
    oracle = c.outputs()
    run = proflib.corpus()
    work = os.path.join(c.dir, 'o')
    for name in sorted(set(ours) | set(oracle)):
        if name.endswith(('.pdf', '.synctex.gz')):
            continue
        check(name in ours and name in oracle, f'only one side has a {os.path.splitext(name)[1]} file')
        if name.endswith('.log'):
            drop = (b'SyncTeX written on',)
            x = [t for _, t in run.normalize_log(ours[name], [work, c.dir]) if not t.startswith(drop)]
            y = [t for _, t in run.normalize_log(oracle[name], [work, c.dir]) if not t.startswith(drop)]
            check(x == y, f'log: {[p for p in zip(x, y) if p[0] != p[1]][:2]}')
        elif name.endswith('.fls'):
            keep = lambda d: [t for t in d.split(b'\n') if b'texmf.cnf' not in t and not t.startswith(b'INPUT /') and b'.fmt' not in t and b'.synctex' not in t]
            check(keep(ours[name]) == keep(oracle[name]), f'fls: {[p for p in zip(keep(ours[name]), keep(oracle[name])) if p[0] != p[1]][:3]}')
        else:
            check(ours[name] == oracle[name], f'{os.path.splitext(name)[1]} differs')


def case_an_error_is_as_under_latexmk(engine):
    """A document with an error: one pass, the status of TeX, and the file:line:error form.  The first pass finds the same .aux as under latexmk, so the log is the same."""
    c = Case('error', engine)
    c.write('a.tex', '\\documentclass{article}\n\\begin{document}\nWords.\n\\nosuchmacro\n\\end{document}\n')
    r = c.direct()
    check(r.returncode == 1 and b'passes=1 result=error' in r.stderr, r.stderr)
    ours = c.outputs()
    check(b'./a.tex:4: Undefined control sequence.' in ours['a.log'], 'file:line:error')
    shutil.rmtree(os.path.join(c.dir, 'o'))
    check(c.latexmk().returncode != 0, 'latexmk')
    same(c.outputs(), ours, 'error')


def case_bibtex_session(engine):
    """A first build asks for bibtex; then typing does not, and neither does a new label, under which latexmk runs bibtex; a new citation does, and an edit of the .bib."""
    c = Case('bibtex', engine)
    c.write('refs.bib', BIB)

    def source(mark):
        return lambda: c.write('a.tex', BIBTEX.replace('MARK', mark))
    steps = [
        ('first', source('Text.'), ['output-missing']),
        ('typing', source('More text.'), []),
        ('a label', source('More text.\\label{new}'), []),
        ('a citation', source('More text.\\label{new} \\cite{beta}'), ['citations-changed']),
        ('typing after it', source('Yet more text.\\label{new} \\cite{beta}'), []),
        ('an edit of the .bib', lambda: c.write('refs.bib', BIB.replace('2001', '2011')), ['source-changed']),
        ('no citation any more', source('Yet more text.\\label{new}'), ['citations-changed']),
    ]
    session(c, steps, 'bibtex')


def case_biber_session(engine):
    """The same with biblatex and biber, if there is a biber."""
    if not shutil.which('biber'):
        print('  no biber here')
        return
    c = Case('biber', engine)
    c.write('refs.bib', BIB)

    def source(mark):
        return lambda: c.write('a.tex', BIBER.replace('MARK', mark))
    steps = [
        ('first', source('Text.'), ['output-missing']),
        ('typing', source('More text.'), []),
        ('a citation', source('More text. \\cite{beta}'), ['bcf-changed']),
        ('typing after it', source('Yet more text. \\cite{beta}'), []),
        ('an edit of the .bib', lambda: c.write('refs.bib', BIB.replace('2001', '2011')), ['source-changed']),
    ]
    session(c, steps, 'biber')


def case_falling_back_to_latexmk_in_the_same_directory(engine):
    """What an editor does with status 10: latexmk, in the same directory.  From then on rstex knows the result is fresh.  The first look at a directory that latexmk made says no-record, once."""
    c = Case('fallback', engine)
    c.write('refs.bib', BIB)
    c.write('a.tex', BIBTEX.replace('MARK', 'Text.'))
    r = c.direct()
    check(r.returncode == common.STATUS_TOOL and b'reason=output-missing' in r.stderr, r.stderr)
    check(c.latexmk().returncode == 0, 'latexmk')
    c.write('a.tex', BIBTEX.replace('MARK', 'More text.'))
    r = c.direct()
    check(r.returncode == 0 and b'tool=' not in r.stderr, r.stderr)
    c.write('a.tex', BIBTEX.replace('MARK', 'More text. \\cite{beta}'))
    r = c.direct()
    check(r.returncode == common.STATUS_TOOL and b'reason=citations-changed' in r.stderr, r.stderr)
    check(c.latexmk().returncode == 0, 'latexmk')
    r = c.direct()
    check(r.returncode == 0 and b'tool=' not in r.stderr and b'passes=1 result=stable' in r.stderr, r.stderr)
    # A directory latexmk made, with nothing of rstex's in it.
    os.remove(os.path.join(c.dir, 'o', 'a.rstex-tools'))
    r = c.direct()
    check(r.returncode == common.STATUS_TOOL and b'reason=no-record' in r.stderr, r.stderr)
    check(c.latexmk().returncode == 0, 'latexmk')
    r = c.direct()
    check(r.returncode == 0 and b'tool=' not in r.stderr, r.stderr)


def case_makeindex_session(engine):
    """The same with an index.  Typing that moves no entry does not ask for makeindex, and a new entry does."""
    c = Case('makeindex', engine)

    def source(mark):
        return lambda: c.write('a.tex', INDEX.replace('MARK', mark))
    steps = [
        ('first', source('Text.'), ['output-missing']),
        ('typing', source('More text.'), []),
        ('an entry', source('More text\\index{entry}.'), ['idx-changed']),
    ]
    session(c, steps, 'makeindex')


def case_index_with_no_printindex(engine):
    """\\makeindex and no \\printindex: no pass wants a .ind, so nothing is asked for, where latexmk runs makeindex."""
    c = Case('index-unread', engine)
    c.write('a.tex', INDEX.replace('\\printindex', ''))
    named, _, code = c.follow()
    check(code == 0 and not named, f'{named} {code}')
    check('a.ind' not in c.outputs() and not os.path.exists(os.path.join(c.dir, 'o', 'a.rstex-tools')), 'files')


def case_tool_record_only_when_needed(engine):
    """A document with no such program has no record, and the record goes when the program does."""
    c = Case('record', engine)
    c.write('refs.bib', BIB)
    c.write('a.tex', BIBTEX.replace('MARK', ''))
    c.follow()
    rec = os.path.join(c.dir, 'o', 'a.rstex-tools')
    check(os.path.exists(rec), 'no record')
    c.write('a.tex', '\\documentclass{article}\n\\begin{document}\nWords.\n\\end{document}\n')
    c.follow()
    check(not os.path.exists(rec), 'the record stayed')


def case_flag_shares_the_checkpoint(engine):
    """With no output directory a pass is a plain run: each takes the checkpoint of the other, and the options do not change its key.  With one, latexmk's search paths are in its key, and they do not disturb each other."""
    c = Case('key', engine)
    c.write('a.tex', REFS)
    c.env['RSTEX_CHECKPOINT_POLICY'] = 'always'
    c.env['RSTEX_CHECKPOINT_TRACE'] = '1'
    r = c.plain(out=None)
    check(r.returncode == 0 and b'restored' not in r.stderr, r.stderr)
    r = c.direct(out=None)
    check(b'rstex: checkpoint: restored' in r.stderr, r.stderr)
    r = c.plain(out=None)
    check(b'rstex: checkpoint: restored' in r.stderr, r.stderr)
    r = c.plain()
    check(r.returncode == 0 and b'restored' not in r.stderr, r.stderr)
    r = c.direct()
    check(r.returncode == 0 and b'rstex: checkpoint: taken' in r.stderr, r.stderr)
    r = c.plain()
    check(b'rstex: checkpoint: restored' in r.stderr, r.stderr)
    r = c.direct()
    check(b'rstex: checkpoint: restored' in r.stderr, r.stderr)


def case_a_file_written_to_the_output_directory_and_input_by_name(engine):
    """\\input gen, of a gen.tex the document wrote: latexmk puts the output directory on TEXINPUTS, and without that this document would fail.  A .bib written there is found too."""
    c = Case('generated', engine)
    c.write('a.tex', r"""\documentclass{article}
\newwrite\w
\immediate\openout\w=gen.tex
\immediate\write\w{Generated text.}
\immediate\closeout\w
\immediate\openout\w=made.bib
\immediate\write\w{@book{made, author={A. Author}, title={Made}, year={2001}, publisher={P}}}
\immediate\closeout\w
\begin{document}
\input{gen}
And \cite{made}.
\bibliographystyle{plain}
\bibliography{made}
\end{document}
""")
    named, _, code = c.follow()
    check(code == 0 and [x[:2] for x in named] == [('bibtex', 'output-missing')], f'{named} {code}')
    ours = c.outputs()
    shutil.rmtree(os.path.join(c.dir, 'o'))
    check(c.latexmk().returncode == 0, 'latexmk')
    got = c.outputs()
    common.latexmk_only(got, ours, 'a')
    same(got, ours, 'generated')


def case_restored_pass_counts_what_the_preamble_read(engine):
    """A file read in the preamble, and written in the body: a restored pass has no read of it to see, but the checkpoint's log says so.  With all of it the same as a cold run."""
    c = Case('preamble-read', engine)
    doc = r"""\documentclass{article}
\newcommand\said{nothing}
\InputIfFileExists{\jobname.cfg}{}{}
\begin{document}
Said: \said.
\newwrite\w
\immediate\openout\w=\jobname.cfg
\immediate\write\w{\string\renewcommand\string\said{VALUE}}
\immediate\closeout\w
\end{document}
"""
    c.write('a.tex', doc.replace('VALUE', 'one'))
    c.env['RSTEX_CHECKPOINT_POLICY'] = 'always'
    r = c.direct()
    check(b'file=o/a.cfg' in r.stderr and b'result=stable' in r.stderr, r.stderr)
    # Now the value changes in the body, and a checkpoint is there to be restored.
    c.write('a.tex', doc.replace('VALUE', 'two'))
    r = c.direct()
    check(r.returncode == 0 and b'file=o/a.cfg' in r.stderr and b'passes=2 result=stable' in r.stderr, r.stderr)
    warm = c.outputs()
    c.env['RSTEX_NO_SNAPSHOT'] = '1'
    shutil.rmtree(os.path.join(c.dir, 'o'))
    r = c.direct()
    check(r.returncode == 0, r.stderr)
    same(warm, c.outputs(), 'restored against cold')


def case_a_pass_from_a_page_record(engine):
    """With no -synctex a pass can resume at a page record, and then all it read before that record is only in the chain's log.  It must still see that a side file it read on the first page has changed, and that a
    name it did not find there has been made, or it stops too soon.  Each result is held against the same document built from nothing with no checkpoint."""
    c = Case('from-a-record', engine)
    pages = ''.join(f'\\section{{Part {k}}}\nWords of part {k}.\n\\clearpage\n' for k in range(12))
    doc = '\\documentclass{article}\n\\begin{document}\n\\InputIfFileExists{gen}{}{}\n\\tableofcontents\n\\clearpage\n' + pages + 'LAST\n\\end{document}\n'
    flags = ('-recorder',)
    c.write('a.tex', doc.replace('LAST', 'The end.'))
    c.env.update(RSTEX_CHECKPOINT_POLICY='always', RSTEX_PAGE_CK='2', RSTEX_CHECKPOINT_TRACE='1')

    def build():
        r = c.direct(lemma=False, extra=flags)
        check(r.returncode == 0, f'status {r.returncode}')
        used = re.findall(rb'pages: \d+ records, using (\d+)', r.stderr)
        passes = common.PASSES_LINE.search(r.stderr)
        return int(used[0]) if used else 0, int(passes.group(1)) if passes else 0

    def from_nothing():
        got = c.outputs()
        keep = os.path.join(c.dir, 'o-kept')
        os.rename(os.path.join(c.dir, 'o'), keep)
        env = dict(c.env, RSTEX_NO_SNAPSHOT='1')
        r = c.direct(lemma=False, extra=flags, env=env)
        check(r.returncode == 0, f'from nothing, status {r.returncode}')
        same(c.outputs(), got, 'against a build from nothing')
        shutil.rmtree(os.path.join(c.dir, 'o'))
        os.rename(keep, os.path.join(c.dir, 'o'))

    # The checkpoint, the chain, and a run that finds it all.
    build()
    build()
    used, passes = build()
    check(used > 0 and passes == 1, f'nothing changed: from record {used}, {passes} passes')
    from_nothing()
    # A part added at the end: the contents on the first page have to see it.
    c.write('a.tex', doc.replace('LAST', '\\section{Added}\nA new part.'))
    used, passes = build()
    check(used > 0 and passes >= 2, f'a part added: from record {used}, {passes} passes')
    from_nothing()
    # Under -recorder a record that fails on the way can send a run cold, which takes the chain with it.
    build()
    # A name looked for on the first page and made on the last.
    c.write('a.tex', doc.replace('LAST', '\\section{Added}\nA new part.\n\\newwrite\\w\\immediate\\openout\\w=gen.tex\n\\immediate\\write\\w{Generated.}\\immediate\\closeout\\w'))
    used, passes = build()
    check(used > 0 and passes >= 2, f'a file made: from record {used}, {passes} passes')
    from_nothing()


def case_the_policy_over_passes(engine):
    """The program's own policy, with nothing in the cache, on a preamble too small to write at first sight: a first pass only leaves a word, a later one writes the checkpoint since it has seen that preamble before, and the one after that
    restores it.  A build later on has the chain and the index as well, and each leaves what a build from nothing leaves."""
    c = Case('policy', engine)
    parts = ''.join(f'\\section{{Part {k}}}\\label{{p{k}}}\nSee page \\pageref{{p{k}}}.\n\n' for k in range(60))
    c.write('a.tex', '\\documentclass{article}\n\\begin{document}\n\\tableofcontents\n' + parts + '\\end{document}\n')
    c.env['RSTEX_CHECKPOINT_TRACE'] = '1'
    r = c.direct(lemma=False)
    m = common.PASSES_LINE.search(r.stderr)
    check(r.returncode == 0 and m and int(m.group(1)) >= 3, r.stderr)
    check(r.stderr.count(b'rstex: checkpoint: taken') == 1 and r.stderr.count(b'rstex: checkpoint: restored') >= 1, r.stderr)
    first_pass_ends = r.stderr.index(b'rstex: until-stable: pass=1')
    check(first_pass_ends < r.stderr.index(b'rstex: checkpoint: taken') < r.stderr.index(b'rstex: checkpoint: restored'), r.stderr)
    first = c.outputs()
    for _ in range(3):
        r = c.direct(lemma=False)
        check(r.returncode == 0 and b'passes=1 result=stable' in r.stderr and b'rstex: checkpoint: restored' in r.stderr, r.stderr)
        same(first, c.outputs(), 'a build with nothing to do')
    env = dict(c.env, RSTEX_NO_SNAPSHOT='1')
    shutil.rmtree(os.path.join(c.dir, 'o'))
    r = c.direct(lemma=False, env=env)
    check(r.returncode == 0, r.stderr)
    same(first, c.outputs(), 'against a build from nothing')


def case_a_change_from_outside_during_a_pass(engine):
    """Something else changes a file the pass has read, here a program it starts: another pass, as latexmk would."""
    c = Case('outside', engine)
    c.write('dep.tex', '\\def\\said{old}\n')
    c.write('change.sh', "test -f done || { touch done; printf '\\\\def\\\\said{new}\\n' > dep.tex; }\n")
    c.write('a.tex', r"""\documentclass{article}
\input{dep}
\immediate\write18{sh change.sh}
\begin{document}
\said.
\end{document}
""")
    r = c.direct(extra=['-shell-escape'])
    check(r.returncode == 0 and b'file=dep.tex' in r.stderr and b'result=stable' in r.stderr, r.stderr)
    check(b'new' in open(os.path.join(c.dir, 'dep.tex'), 'rb').read(), 'dep')


def case_names_with_spaces(engine):
    """A file name with a space, the job's name and what the lines say."""
    c = Case('spaces', engine)
    c.write('refs.bib', BIB)
    c.write('my doc.tex', BIBTEX.replace('MARK', ''))
    r = c.direct(main='my doc.tex', out='out dir')
    check(r.returncode == common.STATUS_TOOL, r.stderr)
    m = common.TOOL_LINE.search(r.stderr)
    check(m and m.group(3) == b'out dir/my doc.aux', r.stderr)
    check(os.path.exists(os.path.join(c.dir, 'out dir', 'my doc.log')), 'log')


def case_plain_tex_and_no_flag(engine):
    """Plain TeX under the flag leaves no .aux, as latexmk would.  Without the flag a LaTeX run leaves what it left, and no record."""
    c = Case('plain', engine)
    c.write('a.tex', 'Words.\\bye\n')
    c.flags = ['-progname=pdftex', '-fmt=' + engine.format_path('pdftex')]
    r = c.direct()
    check(r.returncode == 0 and b'passes=1 result=stable' in r.stderr, r.stderr)
    check('a.aux' not in c.outputs(), 'an .aux stayed')
    c = Case('no-flag', engine)
    c.write('a.tex', REFS)
    r = c.plain()
    check(r.returncode == 0 and b'until-stable' not in r.stderr and b'until-stable' not in r.stdout, r.stderr)
    log = c.outputs()['a.log']
    check(b'No file a.aux.' in log, 'without the flag there is no .aux to find on a first run')


def case_simple_document_needs_one_pass(engine):
    """With no .aux, a one-page document with no references settles in one pass."""
    c = Case('one-pass', engine)
    c.write('a.tex', '\\documentclass{article}\n\\begin{document}\nWords.\n\\end{document}\n')
    r = c.direct()
    check(r.returncode == 0 and b'passes=1 result=stable' in r.stderr, r.stderr)
    log = c.outputs()['a.log']
    check(b'No file a.aux.' not in log, 'the log says what it says under latexmk')


CASES = {k[5:]: v for k, v in sorted(globals().items()) if k.startswith('case_')}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--engine', required=True)
    ap.add_argument('--only', action='append', default=[])
    args = ap.parse_args()
    engine = proflib.Engine('engine', args.engine)
    failed = []
    for name, fn in CASES.items():
        if args.only and name not in args.only:
            continue
        try:
            fn(engine)
            print(f'ok {name}', flush=True)
        except Failed as e:
            failed.append(name)
            print(f'FAILED {name}: {e}', flush=True)
    print(f'{len(CASES) - len(failed) if not args.only else len(args.only) - len(failed)} passed, {len(failed)} failed')
    print('FAIL' if failed else 'PASS')
    sys.exit(1 if failed else 0)


if __name__ == '__main__':
    main()
