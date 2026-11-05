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
        self.lemma_flags = list(common.LEMMA_FLAGS)

    def write(self, name, text):
        p = os.path.join(self.dir, name)
        os.makedirs(os.path.dirname(p), exist_ok=True)
        with open(p, 'w', encoding='utf-8') as f:
            f.write(text)

    def direct(self, main='a.tex', out='o', extra=(), env=None, lemma=True):
        os.makedirs(os.path.join(self.dir, out or '.'), exist_ok=True)
        argv = [self.engine.path] + self.flags + ['--until-stable'] + (self.lemma_flags if lemma else []) + list(extra) + (['-output-directory=' + out] if out else []) + ['./' + main]
        return subprocess.run(argv, cwd=self.dir, env=env or self.env, stdin=subprocess.DEVNULL, capture_output=True, timeout=600, check=False)

    def plain(self, main='a.tex', out='o', extra=(), env=None):
        os.makedirs(os.path.join(self.dir, out or '.'), exist_ok=True)
        argv = [self.engine.path] + self.flags + common.LEMMA_FLAGS + list(extra) + (['-output-directory=' + out] if out else []) + ['./' + main]
        return subprocess.run(argv, cwd=self.dir, env=env or self.env, stdin=subprocess.DEVNULL, capture_output=True, timeout=600, check=False)

    def latexmk(self, main='a.tex', out='o', tex=None):
        cmd = tex or f'{self.engine.path} ' + ' '.join(self.flags) + ' %O %S'
        os.makedirs(os.path.join(self.dir, out), exist_ok=True)
        argv = ['latexmk', '-norc', '-pdf', '-pdflatex=' + cmd] + self.lemma_flags + ['-outdir=' + out, './' + main]
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


def both_ways(c, step, prepare=None, main='a.tex'):
    """One save built by latexmk and by a caller of `--until-stable`, each from what its own last build left, at one path in turn.  What latexmk says, what it left, and what the other took and left."""
    a, b = os.path.join(c.dir, 'o'), os.path.join(c.dir, 'o-latexmk')
    os.makedirs(a, exist_ok=True)
    os.makedirs(b, exist_ok=True)
    # Stamps have to move, whichever way the two run.
    time.sleep(0.02)
    if prepare:
        prepare()
    os.rename(a, a + '-direct')
    os.rename(b, a)
    r = c.latexmk(main=main)
    got_latexmk = c.outputs()
    os.rename(a, b)
    os.rename(a + '-direct', a)
    if prepare:
        prepare()
    named, passes, code = c.follow(main=main)
    got = c.outputs()
    common.latexmk_only(got_latexmk, got, os.path.splitext(main)[0])
    same(got_latexmk, got, step)
    return r, named, passes, code


def against_latexmk(c, steps, prepare=None, main='a.tex'):
    """Saves, some of which end in an error.  Each is (name, edit, what is named, status): the status is latexmk's 12 as rstex says it, and the files are the same at each.  A save that changes nothing and
    has no program to run is not one of them, since latexmk then runs nothing and a pass would be one more."""
    for name, edit, want, status in steps:
        if edit:
            edit()
        r, named, passes, code = both_ways(c, name, prepare, main)
        check(code == status, f'{name}: says {code}, wanted {status}')
        check(r.returncode == ({0: 0, 1: 12, 11: 12}[status]), f'{name}: latexmk says {r.returncode}')
        check(named == want, f'{name}: named {named}, wanted {want}')


def case_after_an_error_makeindex(engine):
    """latexmk's next run after a pass that failed does makeindex, then a pass, whatever has changed.  A first build stops at the error with no program, and from then on the index is what any later
    pass reads: the beamer manual and lshort, on the benchmark, in small."""
    c = Case('error-makeindex', engine)
    source = lambda body: lambda: c.write('a.tex', INDEX.replace('MARK', body))    # noqa: E731
    bad = '\\nosuchmacro'
    against_latexmk(c, [
        ('the first build', source(bad), [], 1),
        ('an edit', source(bad + ' Words.'), [('makeindex', 'output-missing')], 1),
        ('another edit', source(bad + ' More words.'), [], 1),
        ('the error gone', source('Words.'), [], 0),
    ])
    check(os.path.exists(os.path.join(c.dir, 'o', 'a.ind')), 'no .ind')


def case_after_an_error_nothing_changed(engine):
    """The same with no edit at all: the program and a pass are latexmk's next run all the same.  After that it has nothing to do."""
    for tool, doc, reason in (('makeindex', INDEX, 'output-missing'), ('bibtex', BIBTEX, 'output-missing')):
        c = Case('error-same-' + tool, engine)
        c.write('refs.bib', BIB)
        c.write('a.tex', doc.replace('MARK', '\\nosuchmacro'))
        against_latexmk(c, [('the first build', None, [], 1), ('again', None, [(tool, reason)], 1)])


def case_after_an_error_bibtex(engine):
    """The same with bibtex."""
    c = Case('error-bibtex', engine)
    c.write('refs.bib', BIB)
    source = lambda body: lambda: c.write('a.tex', BIBTEX.replace('MARK', body))    # noqa: E731
    bad = '\\nosuchmacro'
    against_latexmk(c, [
        ('the first build', source(bad), [], 1),
        ('an edit', source(bad + ' Words.'), [('bibtex', 'output-missing')], 1),
        ('a citation under the error', source(bad + ' Words \\cite{beta}.'), [], 1),
        ('the error gone', source('Words \\cite{beta}.'), [], 0),
    ])


def case_after_an_error_biber(engine):
    """The same with biblatex and biber, if there is a biber."""
    if not shutil.which('biber'):
        print('  no biber here')
        return
    c = Case('error-biber', engine)
    c.write('refs.bib', BIB)
    source = lambda body: lambda: c.write('a.tex', BIBER.replace('MARK', body))    # noqa: E731
    bad = '\\nosuchmacro'
    against_latexmk(c, [
        ('the first build', source(bad), [], 1),
        ('an edit', source(bad + ' Words.'), [('biber', 'output-missing')], 1),
        ('another edit', source(bad + ' More words.'), [], 1),
        ('the error gone', source('Words.'), [], 0),
    ])


def case_a_pass_that_stops_before_the_index(engine):
    """An error that ends the job before \\printindex: the .idx it wrote is enough for latexmk's next run to do makeindex, whose result the pass after that reads, with another error in it.  A document with no
    \\printindex at all is the same under an error, though not otherwise: see case_index_with_no_printindex."""
    c = Case('error-before-index', engine)
    source = lambda body, end: lambda: c.write('a.tex', INDEX.replace('MARK', body).replace('\\printindex', end))    # noqa: E731
    against_latexmk(c, [
        ('the job ends early', source('\\input{nosuchfile}', '\\printindex'), [], 1),
        ('another error, later', source('\\nosuchmacro', '\\printindex'), [('makeindex', 'output-missing')], 1),
    ])
    with open(os.path.join(c.dir, 'o', 'a.log'), 'rb') as f:
        check(b'a.ind' in f.read(), 'the pass did not read the index')
    c = Case('error-no-printindex', engine)
    source = lambda body: lambda: c.write('a.tex', INDEX.replace('MARK', body).replace('\\printindex', ''))    # noqa: E731
    against_latexmk(c, [
        ('the first build', source('\\nosuchmacro'), [], 1),
        ('an edit', source('\\nosuchmacro More.'), [('makeindex', 'output-missing')], 1),
    ])


def case_error_gone_at_the_next_save(engine):
    """The error goes in the very save that follows it: under latexmk the program still comes before the pass."""
    c = Case('error-gone', engine)
    source = lambda body: lambda: c.write('a.tex', INDEX.replace('MARK', body))    # noqa: E731
    against_latexmk(c, [
        ('the first build', source('\\nosuchmacro'), [], 1),
        ('the error gone', source('Words.'), [('makeindex', 'output-missing')], 0),
        ('an edit', source('More words.'), [], 0),
    ])


def case_stale_under_an_error_counts_as_seen(engine):
    """latexmk's rdb_update_gen_files: a new index entry in a save with an error runs no makeindex, then or when the error goes, as the .idx is what it last saw.  Only the next change does.  Until then the
    index lacks the entry, under latexmk as here."""
    c = Case('error-stale', engine)
    source = lambda mark: lambda: c.write('a.tex', INDEX.replace('MARK', mark))    # noqa: E731
    against_latexmk(c, [
        ('the first build', source('Text.'), [('makeindex', 'output-missing')], 0),
        ('an entry and an error', source('Text\\index{entry}. \\nosuchmacro'), [], 1),
        ('the error gone', source('Text\\index{entry}.'), [], 0),
        ('another entry', source('Text\\index{entry}\\index{other}.'), [('makeindex', 'idx-changed')], 0),
    ])
    with open(os.path.join(c.dir, 'o', 'a.ind'), encoding='utf-8') as f:
        check('other' in f.read(), 'the .ind')


def case_a_bib_file_edited_under_an_error(engine):
    """A .bib file changed since bibtex ran comes first under latexmk, whether or not the pass will fail, so that a pass with an error shows it too."""
    c = Case('error-bib-edited', engine)
    c.write('refs.bib', BIB)
    source = lambda mark: lambda: c.write('a.tex', BIBTEX.replace('MARK', mark))    # noqa: E731
    against_latexmk(c, [
        ('the first build', source('Text.'), [('bibtex', 'output-missing')], 0),
        ('an error', source('Text. \\nosuchmacro'), [], 1),
        ('the .bib', lambda: c.write('refs.bib', BIB.replace('2001', '2011')), [('bibtex', 'source-changed')], 1),
        ('an edit', source('More. \\nosuchmacro'), [], 1),
        ('the error gone', source('More.'), [], 0),
    ])


def case_no_bib_file_under_an_error(engine):
    """latexmk's $bibtex_use = 1: no .bib file, no bibtex, under an error too.  It runs first in the save that brings the file."""
    c = Case('error-no-bib', engine)
    source = lambda mark: lambda: c.write('a.tex', BIBTEX.replace('MARK', mark))    # noqa: E731
    against_latexmk(c, [
        ('the first build', source('\\nosuchmacro'), [], 1),
        ('an edit', source('\\nosuchmacro More.'), [], 1),
        ('the file comes', lambda: c.write('refs.bib', BIB), [('bibtex', 'output-missing')], 1),
    ])


def case_a_cut_bcf_goes_aside(engine):
    """A pass that dies in the body of a biblatex document leaves a .bcf cut short, which latexmk renames, with any .bbl.  The same names and the same bytes here."""
    c = Case('error-cut-bcf', engine)
    c.write('refs.bib', BIB)
    c.write('a.tex', BIBER.replace('MARK', '\\input{nosuchfile}'))
    against_latexmk(c, [('the first build', None, [], 1)])
    check('a.bcf-SAVE-ERROR' in c.outputs() and 'a.bcf' not in c.outputs(), 'the .bcf')


def case_a_directory_a_pass_missed(engine):
    """\\include of a file whose directory the output directory has not.  latexmk makes it and goes on, where its log has `! I can't write on file`.  Under -file-line-error that line reads otherwise, it
    does not see it, and the error stands.  So the flags Lemma passes leave this to Lemma, as ever."""
    doc = '\\documentclass{article}\n\\begin{document}\n\\include{sub/ch}\nBack.\n\\end{document}\n'
    c = Case('error-directory', engine)
    c.write('sub/ch.tex', 'A chapter.\n')
    c.write('a.tex', doc)
    c.lemma_flags = ['-interaction=nonstopmode', '-recorder']
    against_latexmk(c, [('the first build', None, [], 0)])
    check(os.path.isdir(os.path.join(c.dir, 'o', 'sub')), 'the directory')
    c = Case('error-directory-fle', engine)
    c.write('sub/ch.tex', 'A chapter.\n')
    c.write('a.tex', doc)
    against_latexmk(c, [('the first build', None, [], 1)])
    check(not os.path.isdir(os.path.join(c.dir, 'o', 'sub')), 'the directory')


def case_a_change_from_outside_under_an_error(engine):
    """latexmk goes on after an error if a file of the user's changed during the pass, which it can tell from its state from before, and so not on a first build.  Here a program the pass starts changes it."""
    c = Case('error-outside', engine)
    doc = r"""\documentclass{article}
\input{dep}
\immediate\write18{sh change.sh}
\begin{document}
\said. MARK
\nosuchmacro
\end{document}
"""
    c.write('change.sh', "test -f go || exit 0; rm go; printf '\\\\def\\\\said{new}\\n' > dep.tex\n")
    c.write('dep.tex', '\\def\\said{old}\n')
    c.lemma_flags = common.LEMMA_FLAGS + ['-shell-escape']
    c.write('a.tex', doc.replace('MARK', ''))
    against_latexmk(c, [('the first build', None, [], 1)])

    def prepare():
        c.write('dep.tex', '\\def\\said{old}\n')
        c.write('go', '')
    c.write('a.tex', doc.replace('MARK', 'Words.'))
    r, named, passes, code = both_ways(c, 'a change during the pass', prepare)
    check(code == 1 and passes == 2 and not named, f'{code}, {passes} passes, named {named}')
    check(r.returncode == 12 and common.RULE_RUNS.findall(r.stdout + r.stderr).count(b'pdflatex') == 2, 'latexmk')


def case_falling_back_to_latexmk_after_an_error(engine):
    """What an editor does with status 10 after an error: latexmk for that save, with no record of its own there.  It runs the program and a pass, gives 12, and the next save is direct again with the index in
    place."""
    c = Case('error-fallback', engine)
    c.write('a.tex', INDEX.replace('MARK', '\\nosuchmacro'))
    r = c.direct()
    check(r.returncode == 1 and b'tool=' not in r.stderr, r.stderr)
    c.write('a.tex', INDEX.replace('MARK', '\\nosuchmacro More.'))
    r = c.direct()
    check(r.returncode == common.STATUS_TOOL and b'reason=output-missing' in r.stderr and b'passes=0 result=tool' in r.stderr, r.stderr)
    r = c.latexmk()
    check(r.returncode == 12, f'latexmk says {r.returncode}')
    check(os.path.exists(os.path.join(c.dir, 'o', 'a.ind')), 'no .ind')
    c.write('a.tex', INDEX.replace('MARK', '\\nosuchmacro More and more.'))
    r = c.direct()
    check(r.returncode == 1 and b'tool=' not in r.stderr and b'passes=1 result=error' in r.stderr, r.stderr)
    c.write('a.tex', INDEX.replace('MARK', 'More and more.'))
    r = c.direct()
    check(r.returncode == 0 and b'tool=' not in r.stderr, r.stderr)


def case_a_document_that_never_settles(engine):
    """A file looked for by \\IfFileExists and written by the same pass is nothing to latexmk on a first build, as neither its .fls nor its log says it was looked for: one run and status 0, with the bytes that
    gives.  A later build reads it, and neither side settles: five passes, latexmk's 12 and rstex's 11.  With \\@input the log has `No file`, and the first build takes two."""
    c = Case('never-settles', engine)
    doc = r"""\documentclass{article}
\newwrite\f
\newcount\n
\IfFileExists{\jobname.cnt}{\input{\jobname.cnt}}{\gdef\cntval{0}}
\n=\cntval\relax\advance\n by 1
\immediate\openout\f=\jobname.cnt
\immediate\write\f{\string\gdef\string\cntval{\the\n}}
\immediate\closeout\f
\begin{document}
Run \the\n.
\end{document}
"""
    c.write('a.tex', doc)
    r, named, passes, code = both_ways(c, 'the first build')
    check(r.returncode == 0 and code == 0 and passes == 1, f'{r.returncode} {code} {passes}')
    c.write('a.tex', doc + '% saved\n')
    r, named, passes, code = both_ways(c, 'a save')
    check(r.returncode == 12 and code == 11 and passes == 5, f'{r.returncode} {code} {passes}')
    check(b'needed too many passes' in r.stdout + r.stderr, 'latexmk')
    c = Case('missed-in-the-log', engine)
    c.write('a.tex', r"""\documentclass{article}
\newwrite\f
\newcommand\said{nothing}
\makeatletter
\@input{\jobname.cfg}
\makeatother
\immediate\openout\f=\jobname.cfg
\immediate\write\f{\string\renewcommand\string\said{value}}
\immediate\closeout\f
\begin{document}
Said \said.
\end{document}
""")
    r, named, passes, code = both_ways(c, 'the first build')
    check(r.returncode == 0 and code == 0 and passes == 2, f'{r.returncode} {code} {passes}')


def case_a_format_being_built_says_so(engine):
    """A program that runs rstex can tell a first run by one line on stderr, with nothing on stdout or in the log, and the next run has no such line."""
    c = Case('format-line', engine)
    c.write('a.tex', '\\documentclass{article}\n\\begin{document}\nWords.\n\\end{document}\n')
    c.flags = ['-progname=pdflatex']
    r = c.direct()
    check(r.returncode == 0, r.stderr)
    check(r.stderr.split(b'\n').count(b'rstex: building the format pdflatex') == 1, r.stderr)
    check(b'building the format' not in r.stdout, 'on stdout')
    check(r.stderr.index(b'building the format') < r.stderr.index(b'passes=1'), 'not first')
    log = c.outputs()['a.log']
    check(b'building the format' not in log, 'in the log')
    r = c.direct()
    check(r.returncode == 0 and b'format' not in r.stderr, r.stderr)
    check(c.outputs()['a.log'] == log, 'the log differs without the build')


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
    """A document with an error: one pass, the status of TeX, no program, and the file:line:error form.  The first pass finds the same .aux as under latexmk, so the log is the same."""
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
    # A file looked for and made: nothing to latexmk on a first build, unless the log says it was missed.  The next pass reads it.
    r = c.direct()
    check(b'passes=1 result=stable' in r.stderr, r.stderr)
    r = c.direct()
    check(b'passes=1 result=stable' in r.stderr, r.stderr)
    # Now the value changes in the body, and a checkpoint is there to be restored.
    c.write('a.tex', doc.replace('VALUE', 'two'))
    start = os.path.join(c.dir, 'o-start')
    shutil.copytree(os.path.join(c.dir, 'o'), start)
    r = c.direct()
    check(r.returncode == 0 and b'file=o/a.cfg' in r.stderr and b'passes=2 result=stable' in r.stderr, r.stderr)
    warm = c.outputs()
    c.env['RSTEX_NO_SNAPSHOT'] = '1'
    shutil.rmtree(os.path.join(c.dir, 'o'))
    os.rename(start, os.path.join(c.dir, 'o'))
    r = c.direct()
    check(r.returncode == 0, r.stderr)
    same(warm, c.outputs(), 'restored against cold')


def case_a_pass_from_a_page_record(engine):
    """With no -synctex a pass can resume at a page record, and then all it read before that record is only in the chain's log.  It must still see that a side file it read on the first page has changed, and that a
    name it did not find there has been made, or it stops too soon.  Each result is held against the same document built from nothing with no checkpoint."""
    c = Case('from-a-record', engine)
    pages = ''.join(f'\\section{{Part {k}}}\nWords of part {k}.\n\\clearpage\n' for k in range(12))
    # \@input, as the log has to say it missed the file for that to count.
    doc = '\\documentclass{article}\n\\begin{document}\n\\makeatletter\\@input{gen.tex}\\makeatother\n\\tableofcontents\n\\clearpage\n' + pages + 'LAST\n\\end{document}\n'
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


def case_names_with_spaces_against_latexmk(engine):
    """\\jobname keeps the quotes of such a name and its files do not: the .aux a first pass finds, the .bcf, the record and the log's `No file` all go by the file's name.  A first pass that fails has latexmk's
    .aux, a document with references takes its second pass, and each program is found."""
    main = 'my doc.tex'
    c = Case('spaces-refs', engine)
    c.write(main, REFS)
    against_latexmk(c, [('the first build', None, [], 0)], main=main)
    c = Case('spaces-error', engine)
    c.write(main, INDEX.replace('MARK', '\\nosuchmacro'))
    against_latexmk(c, [('the first build', None, [], 1), ('again', None, [('makeindex', 'output-missing')], 1)], main=main)
    check(os.path.exists(os.path.join(c.dir, 'o', 'my doc.rstex-tools')), 'the record has the name of the files')
    c = Case('spaces-bibtex', engine)
    c.write('refs.bib', BIB)
    c.write(main, BIBTEX.replace('MARK', 'Text.'))
    against_latexmk(c, [('the first build', None, [('bibtex', 'output-missing')], 0)], main=main)
    if shutil.which('biber'):
        c = Case('spaces-biber', engine)
        c.write('refs.bib', BIB)
        c.write(main, BIBER.replace('MARK', 'Text.'))
        against_latexmk(c, [('the first build', None, [('biber', 'output-missing')], 0)], main=main)


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
