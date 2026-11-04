#!/usr/bin/env -S python3 -I
r"""Are page records sound?  crates/engine/src/checkpoint/pages.rs.  A mistake there is silently wrong output after an edit, so all of this asks one question: does a run that resumes give what a cold run gives?

    python3 -I tests/corpus/snapshot/pages.py gate --engine BIN [-j N]              all that follows on a document made here, and on public documents
    python3 -I tests/corpus/snapshot/pages.py records --engine BIN                  a chain on the document made here, and a resume from each record
    python3 -I tests/corpus/snapshot/pages.py world --engine BIN [-j N]             the world changes between runs, in every way that document can see
    python3 -I tests/corpus/snapshot/pages.py damage --engine BIN                   bytes flipped and files cut in a chain
    python3 -I tests/corpus/snapshot/pages.py kill --engine BIN [--doc ID] [--trials N]    SIGKILL at a random moment, also while a resumed run cuts and appends
    python3 -I tests/corpus/snapshot/pages.py together --engine BIN                 runs at once on one chain
    python3 -I tests/corpus/snapshot/pages.py clock --engine BIN [--wait S]         a chain under the real clock, used a minute later
    python3 -I tests/corpus/snapshot/pages.py session --engine BIN --doc ID [--steps N] [--every N] [--seed S] [--mix letter,word,para]    a sitting at the editor, to the fixed point
    python3 -I tests/corpus/snapshot/pages.py verify --engine BIN --docs ID,ID      the state itself, against each later record
    python3 -I tests/corpus/snapshot/pages.py reclaim --engine BIN                  on a build with the feature tight_heap: a heap that runs out of bump space, resumed

--keep-pdf has a run find the PDF that the last one left, as between two runs of a document, and not only the side files: so the output cache of DESIGN section 9 is at work in all of the above, and a run from a record
has to take what comes after it from there.  A session always does.

BIN is a binary: this finds its format as tools/prof/ does.  A document is a manifest id.  Edits are generated, into a tree of links under target/, and the document's directory is never written to.
What is printed is numbers and ids.  Everything runs under target/.  Exit status 0 if nothing differs.  Every run is judged on the PDF, the log and every side file, the standard output and the exit status.
"""
import argparse
import concurrent.futures as cf
import importlib.util
import os
import random
import re
import secrets
import shutil
import signal
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(os.path.dirname(HERE)))
sys.path.insert(0, os.path.join(REPO, 'tools', 'prof'))
import proflib  # noqa: E402

spec = importlib.util.spec_from_file_location('run_snapshot', os.path.join(HERE, 'run_snapshot.py'))
snapshot_tool = importlib.util.module_from_spec(spec)
spec.loader.exec_module(snapshot_tool)
ROOT = os.path.join(REPO, 'target', 'corpus', 'snapshot', 'pages')
JPEG = snapshot_tool.JPEG
PROSE = re.compile(rb"^[A-Za-z][A-Za-z ,.;:'-]{30,}$")
PARA = (b'\nthe quick brown fox jumps over the lazy dog and then it goes on for some time with no more to say about\n'
        b'what the dog thought of it all or where the fox went after that which is a question for another day and\n'
        b'another document entirely since this one only needs a paragraph of a certain length to push what follows\n'
        b'further down the page and perhaps over to the next one where it will have to find a new place to break\n\n')
SIDE = ('.aux', '.toc', '.out', '.lof', '.lot', '.bbl', '.idx', '.nav', '.snm', '.vrb', '.thm', '.loe')
# --keep-pdf.
KEEP_PDF = False

# As much as a document can hold across a page break.  Each \clearpage is a place for a record, and what is set up before one is used after it.
ADV = r'''\documentclass{article}
\usepackage[T1]{fontenc}
\usepackage{graphicx}
\usepackage{xcolor}
\usepackage{microtype}
\usepackage[pdfusetitle]{hyperref}
\newwrite\wa \newwrite\wb \newread\ra \newread\rb \newread\rc
\pdfgentounicode=1
\newcommand\filler[1]{\par #1 The quick brown fox jumps over the lazy dog and then it goes on for some time, with $x^2+y^2=z^2$ inline.
What the dog thought of it all is a question for another day and another document entirely.\par}
\begin{document}
\IfFileExists{absent0.tex}{\input{absent0}}{not there at the start}
\tableofcontents
\clearpage
\section{Match and streams}
\label{sec:one}
The fox came back the next day and found that nothing had changed at all.
\pdfmatch{b+}{aabbcc}
\immediate\openout\wa=side.out
\immediate\write\wa{\string\def\string\fromside{immediate one}}
\write\wa{\string\def\string\fromdeferred{deferred \thepage}}
\openin\ra=data.txt
\read\ra to\linea \linea
\color{red}
\filler{Red starts.}
\clearpage
\section{Map line and glyph unicode}
\pdfmapline{=cmss10 CMSS10 <cmss10.pfb}
\pdfglyphtounicode{a}{0061}
{\fontencoding{OT1}\fontfamily{cmss}\selectfont sans text here \filler{Sans.}}
\read\ra to\lineb \lineb
\clearpage
\section{PNG named here and used two pages on}
\pdfximage width 3cm {pic.png}\xdef\pngref{\the\pdflastximage}
\pdfximage width 2cm {alpha.png}\xdef\alpharef{\the\pdflastximage}
\filler{Colour goes on.}
\clearpage
\filler{Nothing used yet.}
\clearpage
\section{JPEG and reuse}
The dog had not moved either, which surprised nobody who knew that dog at all.
\pdfrefximage\pngref\relax
\pdfximage width 2cm {pic.jpg}\xdef\jpgref{\the\pdflastximage}
\pdfrefximage\jpgref\relax \pdfrefximage\pngref\relax \pdfrefximage\alpharef\relax
\normalcolor
\clearpage
\section{Last match, sums}
\pdflastmatch 0 \ifnum\pdfshellescape=0 none\fi
\pdfmdfivesum file {data2.txt} and \pdffilesize{data3.txt} and \pdffilemoddate{data3.txt} and \pdffiledump offset 0 length 4 {data2.txt}
\filler{Sums.}
\clearpage
\section{PDF named here and used on the next page and later}
\pdfximage page 1 {pic.pdf}\xdef\pdfrefa{\the\pdflastximage}
\filler{A file open across this page.}
\clearpage
\pdfrefximage\pdfrefa\relax
\filler{After a figure.}
\clearpage
\section{The same file again, which has to be copied again}
\filler{Later.}
\includegraphics[width=2cm,page=1]{pic.pdf}
\clearpage
\section{Forms, objects, links over a page break}
Some days later it was still the same story with the same two animals in it.
\setbox0\hbox{form text \textbf{bold}}\pdfxform 0\xdef\formref{\the\pdflastxform}
\pdfobj stream {a stream object}\pdfrefobj\pdflastobj
\pdfobj file {data2.txt}\pdfrefobj\pdflastobj
\pdfobj reserveobjnum \xdef\resobj{\the\pdflastobj}
\pdfrefxform\formref\relax
\pdfoutline user{/S/URI/URI(http://example.org)} count 0 {An outline}
\pdfdest name {mydest} xyz
\leavevmode\pdfstartlink attr{/Border[0 0 0]} goto name {mydest}\relax linked text\pdfendlink
\filler{A link.}
\leavevmode\pdfsavepos\the\pdflastxpos\ \the\pdflastypos
\clearpage
\pdfobj useobjnum \resobj {(filled later)}
\section{Inside scantokens and an input file}
\begingroup\everyeof{\noexpand}\endlinechar=-1 \scantokens{\filler{in scantokens}\clearpage \filler{still in scantokens}}\endgroup
\input{sub}
\filler{Back from sub.}
\clearpage
\section{Fonts made in the body}
\font\xx=cmr12 at 13pt \xx some text at 13 \normalfont
\letterspacefont\yy\xx 100 \yy spaced \normalfont
\pdfcopyfont\zz\xx \zz copied \normalfont
\pdfincludechars\xx{QXZ}
\pdffontattr\xx{/MyAttr true}
\pdfadjustspacing=2 \pdfprotrudechars=2
\filler{Expansion on.}
\clearpage
\section{Clock and lookups}
\the\year-\the\month-\the\day\ \today
\IfFileExists{absent.tex}{\input{absent}}{not there}
\IfFileExists{shadow.tex}{\input{shadow}}{not there}
\filler{Clock.}
\read\ra to\linec \linec
\clearpage
\section{A long stream, open across pages and read while it is written}
\immediate\openout\wb=long.out
\count255=0 \loop\advance\count255 by 1 \immediate\write\wb{a line of a stream that is going to be longer than any buffer, number \the\count255}\ifnum\count255<120 \repeat
\filler{Written.}
\clearpage
\immediate\write\wb{one line more}
\filler{And a little more.}
\clearpage
\section{The same name twice}
\immediate\closeout\wb
\immediate\openout\wb=long.out
\immediate\write\wb{a shorter one}
\immediate\closeout\wb
\input{long.out}
\clearpage
\section{The last line of a file with no end of line}
\openin\rc=noend.txt
\read\rc to\lined \lined
\filler{That was the end of it.}
\clearpage
\read\rc to\linee \linee
\section{A seed set here and read later}
\pdfsetrandomseed 42
\filler{Seeded.}
\clearpage
\the\pdfuniformdeviate 1000
\filler{That read the generator, so this is the last of the records.}
\clearpage
\section{Closing}
At the end of it all the fox went home and the dog went on sitting where it sat.
\closeout\wa
\closein\ra
\filler{Closed.}
\clearpage
\section{Read back what was written}
\input{side.out}
\fromside
\ifdefined\fromdeferred\fromdeferred\else no deferred\fi
\pageref{sec:one}
\filler{Read back.}
\pdfinfo{/Keywords (adversarial)}
\pdfcatalog{/PageMode /UseOutlines}
\pdfnames{/MyName (x)}
\pdftrailer{/MyTrailer true}
\pdfpageattr{/Rotate 0}
\clearpage
\filler{The last page.}
\end{document}
'''
INCLUDE = r"""\documentclass{book}
\newcommand\filler[1]{\par #1 The quick brown fox jumps over the lazy dog and then it goes on for some time, with $x^2+y^2=z^2$ inline.\par}
\begin{document}
\tableofcontents
\include{ch1}
\include{ch2}
\include{ch3}
\end{document}
"""
CHAPTER = r"""\chapter{Chapter %(n)d}\label{ch:%(n)d}
A line of chapter %(n)d that can be edited with no harm to anything at all.
\filler{One.}\clearpage\filler{Two.}\clearpage\filler{Three.}\clearpage\filler{Four.}\clearpage
\section{More of chapter %(n)d}
See chapter \ref{ch:1} on page \pageref{ch:1}.
\filler{Five.}\clearpage\filler{Six.}
"""
SUB = '\\filler{In sub, first.}\n\\clearpage\n\\filler{In sub, second.}\n\\clearpage\n\\filler{In sub, third.}\n\\endinput\nThis line is never read.\n'
# A group around a page, with the clock's parameters set in it, and an error on the way.
GROUPS = r'''\documentclass{article}
\newcommand\filler[1]{\par #1 The quick brown fox jumps over the lazy dog and then it goes on for some time, with $x^2+y^2=z^2$ inline.\par}
\begin{document}
\filler{First.}
\clearpage
\filler{Second.}
\clearpage
\filler{Third.}
\clearpage
{\year=1999 \month=3 \filler{In a group that changes the date.}
\clearpage
\filler{Still in it.}
\clearpage
\filler{And the fourth page of that.} \the\year
\clearpage}
\the\year
\filler{After it.}
\clearpage
\filler{The last.}
\end{document}
'''
ERRORS = r'''\documentclass{article}
\newcommand\filler[1]{\par #1 The quick brown fox jumps over the lazy dog and then it goes on for some time, with $x^2+y^2=z^2$ inline.\par}
\begin{document}
\filler{First.}
\clearpage
\undefinedcommandhere
\filler{An error was on this page.}
\clearpage
\filler{A page past the error.}
\clearpage
\filler{And another.} \badbox{here}
\clearpage
\hbox to 1pt{An overfull box and more words}\filler{Another still.}
\clearpage
\filler{The last.}
\end{document}
'''


# The page builder's sums past 2^31 where a page goes out, and back under it after the record: page_so_far and insert_penalties are wide, and what a resumed run reads has to be the whole sum.
WIDE = r'''\documentclass{article}
\begin{document}
\output={\setbox0\box255 \setbox0\box200 \deadcycles=0 }
\skip200=0pt \dimen200=10pt \count200=1000 \vsize=16383pt
\def\up{\vskip 0pt plus 12000pt minus 12000pt }
\def\down{\vskip 0pt plus -12000pt minus -12000pt }
\hrule height 1pt
\insert200{\hrule height 20pt \penalty 7 \hrule height 20pt}
\up\up\up
\floatingpenalty=-2000000000
\insert200{\hrule height 5pt}
\insert200{\hrule height 5pt}
\penalty0
\shipout\hbox{first}
\kern 12000pt \kern 12000pt \kern 12000pt
\penalty0
\shipout\hbox{second}
\down\down
\kern -12000pt \kern -12000pt
\floatingpenalty=2000000000
\insert200{\hrule height 5pt}
\insert200{\hrule height 5pt}
\penalty0
\shipout\hbox{third}
\showthe\pagestretch
\showthe\pageshrink
\showthe\pagetotal
\showthe\insertpenalties
\showlists
\penalty-10000
\end{document}
'''


# Lists of one length after another on each page, as tests/integration/toks/heap_classes.tex, and a few that stay.  What this makes in all is far beyond the heap of a tight_heap build, so a run that ends has reclaimed.
RECLAIM = r'''\documentclass{article}
\newcount\n \newcount\k \newcount\m \newtoks\this
\def\phase{%
  \this=\expandafter{\romannumeral\numexpr\k*1000\relax}%
  \expandafter\edef\csname keep\the\k\endcsname{\the\k\the\this}%
  \m=\numexpr 60000/(\k+2)\relax
  \n=0
  \loop\ifnum\n<\m \expandafter\edef\csname x\the\n\endcsname{\the\this}\advance\n1 \repeat
  \n=0
  \loop\ifnum\n<\m \expandafter\let\csname x\the\n\endcsname\relax \advance\n1 \repeat}
\def\grow{\k=\numexpr\k+\k/8+1\relax}
\begin{document}
\k=70
\def\run{\ifnum\k<9000 \phase Page \the\k, with \pdfmdfivesum{\the\this}.\clearpage\grow \expandafter\run\fi}\run
\k=70
\def\check{\ifnum\k<9000 \pdfmdfivesum{\detokenize\expandafter\expandafter\expandafter{\csname keep\the\k\endcsname}} \grow \expandafter\check\fi}\check
\end{document}
'''


def write_adv(src):
    os.makedirs(src, exist_ok=True)
    for name, text in (('adv.tex', ADV), ('sub.tex', SUB), ('groups.tex', GROUPS), ('errors.tex', ERRORS), ('wide.tex', WIDE), ('reclaim.tex', RECLAIM), ('include.tex', INCLUDE), ('ch1.tex', CHAPTER % {'n': 1}), ('ch2.tex', CHAPTER % {'n': 2}), ('ch3.tex', CHAPTER % {'n': 3}),
                       ('data.txt', 'line one of the data\n\nline three of the data\n\nline five of the data\n\nline seven\n'),
                       ('data2.txt', 'data two contents\n'), ('data3.txt', 'data three contents\n'), ('noend.txt', 'the only line, with no end')):
        with open(os.path.join(src, name), 'w') as f:
            f.write(text)
    for name, data in (('pic.png', snapshot_tool.tiny_png(8, 8, 100)), ('alpha.png', snapshot_tool.tiny_png(8, 8, 180, alpha=True)), ('pic.pdf', snapshot_tool.tiny_pdf(72))):
        with open(os.path.join(src, name), 'wb') as f:
            f.write(data)
    shutil.copy(JPEG, os.path.join(src, 'pic.jpg'))


def fresh(tag):
    d = os.path.join(ROOT, f'{tag}-{secrets.token_hex(4)}')
    os.makedirs(d)
    return d


def state(d):
    res = {}
    for root, _, names in os.walk(d):
        for n in names:
            p = os.path.join(root, n)
            with open(p, 'rb') as f:
                res[os.path.relpath(p, d)] = f.read()
    return res


def differs(a, b):
    """Numbers and the kinds of files only: a document's bytes are not for a report."""
    out = []
    for name in sorted(set(a) | set(b)):
        if a.get(name) != b.get(name):
            ext = name.rsplit('.', 1)[-1]
            if name not in a or name not in b:
                out.append(ext + ':absent')
            else:
                x, y = a[name], b[name]
                n = min(len(x), len(y))
                k = next((i for i in range(n) if x[i] != y[i]), n)
                out.append(f'{ext}:{len(x)}/{len(y)}@{k}')
    return out


STAT_LINE = re.compile(rb'^ *[0-9,in ]+ *(strings|string characters|words of memory|multiletter|words of font|hyphenation|PDF objects|compressed objects|named dest|words of extra|stack positions)|^ *[0-9inbpsf,]+ stack positions')


def where_logs_differ(a, b):
    """Of two logs, which lines differ: their numbers, how long they are, and the line itself only if it is one of the statistics at the end, which say nothing of a document."""
    x, y = a.split(b'\n'), b.split(b'\n')
    out = [f'{len(x)}/{len(y)} lines']
    for k in range(max(len(x), len(y))):
        p = x[k] if k < len(x) else None
        q = y[k] if k < len(y) else None
        if p != q:
            show = lambda t: None if t is None else (t.decode('latin-1') if STAT_LINE.match(t) else f'<{len(t)} bytes>')  # noqa: E731
            out.append((k + 1, show(p), show(q)))
        if len(out) > 8:
            break
    return out


def put(d, st):
    shutil.rmtree(d, ignore_errors=True)
    os.makedirs(d)
    for k, v in st.items():
        p = os.path.join(d, k)
        os.makedirs(os.path.dirname(p), exist_ok=True)
        with open(p, 'wb') as f:
            f.write(v)


def put_out(d, st):
    """A run's output directory as `st` has it, and under --keep-pdf with the PDF as it stands."""
    kept = {}
    if KEEP_PDF and os.path.isdir(d):
        for n in os.listdir(d):
            if n.endswith('.pdf'):
                with open(os.path.join(d, n), 'rb') as f:
                    kept[n] = f.read()
    put(d, dict(st, **kept))


def said(stderr):
    pre = 'rstex: checkpoint: '
    return [line[len(pre):] for line in stderr.decode('latin-1').splitlines() if line.startswith(pre)]


class World:
    """A directory of sources, one of outputs, and a cache, with what a run needs."""

    def __init__(self, engine, tag, main='adv.tex', extra_args=(), in_place=False):
        self.engine = engine
        self.root = fresh(tag)
        self.src, self.out, self.cache = self.root + '/src', self.root + '/o', self.root + '/cache'
        self.in_place = in_place
        write_adv(self.src)
        os.makedirs(self.out)
        os.makedirs(self.cache)
        c = proflib.corpus()
        self.env = {k: os.environ[k] for k in ('PATH', 'HOME') if k in os.environ}
        self.env.update(c.DETERMINISM)
        self.env.update(c.NO_MKTEX)
        self.env.update(c.HERMETIC)
        self.env.update(openout_any='p', XDG_CACHE_HOME=self.cache, RSTEX_CHECKPOINT_POLICY='always')
        self.main = main
        self.argv = [engine.path, '-progname=pdflatex', '-fmt=' + engine.format_path('pdflatex'), '-interaction=batchmode', '-no-shell-escape', '-output-directory=' + self.out, *extra_args, main]
        self.world0 = state(self.src)
        self.pre = {}
        if in_place:
            # Outputs among the sources, with no -output-directory.
            self.argv.remove('-output-directory=' + self.out)
            self.out = self.src

    def run(self, **extra):
        env = dict(self.env, **extra)
        p = subprocess.run(self.argv, cwd=self.src, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        return p

    def cold(self, **extra):
        return self.run(RSTEX_NO_SNAPSHOT='1', **extra)

    def settle(self):
        """Side files at the fixed point, which each run starts from."""
        prev = None
        for _ in range(6):
            self.cold()
            s = {k: v for k, v in state(self.out).items() if k.endswith(SIDE)}
            if s == prev:
                break
            prev = s
        self.pre = prev or {}

    def reset_out(self):
        if self.in_place:
            for k in set(state(self.src)) - set(self.world0):
                if not (KEEP_PDF and k.endswith('.pdf')):
                    os.unlink(os.path.join(self.src, k))
            for k, v in self.pre.items():
                with open(os.path.join(self.src, k), 'wb') as f:
                    f.write(v)
        else:
            put_out(self.out, self.pre)

    def reset_world(self):
        now = state(self.src)
        for k in set(now) - set(self.world0):
            os.unlink(os.path.join(self.src, k))
        for k, v in self.world0.items():
            if now.get(k) != v:
                with open(os.path.join(self.src, k), 'wb') as f:
                    f.write(v)

    def ck_dir(self):
        return self.cache + '/rstex/checkpoints'

    def chain_dir(self):
        d = self.ck_dir()
        for n in sorted(os.listdir(d)) if os.path.isdir(d) else []:
            if n.endswith('.pg'):
                return os.path.join(d, n)
        return None

    def write_chain(self, **env):
        """The run that writes a preamble checkpoint, and the one that restores it and writes a chain.  What that second one said."""
        self.reset_out()
        self.run(RSTEX_NO_PAGES='1')
        self.reset_out()
        return self.run(RSTEX_CHECKPOINT_TRACE='1', RSTEX_PAGE_INPUTS='1', **env)

    def remove(self):
        shutil.rmtree(self.root, ignore_errors=True)


def judged(ref, ref_run, got, run):
    d = differs(ref, got)
    if run.stdout != ref_run.stdout:
        d.append('standard output')
    if run.returncode != ref_run.returncode:
        d.append(f'exit status {ref_run.returncode} and {run.returncode}')
    return d


def record_lines(p):
    where = []
    for line in said(p.stderr):
        m = re.match(r'pages: rec (\d+) page (\d+) at (\d+) (.*)$', line)
        if m:
            where.append((int(m.group(1)), int(m.group(2)), int(m.group(3)), m.group(4)))
    return where


def used_record(p):
    for line in said(p.stderr):
        m = re.match(r'pages: (\d+) records, using (\d+)', line)
        if m:
            return int(m.group(2))
    return 0


def records(args, engine, main='adv.tex', extra_args=(), every='1', least=1, env=None, in_place=False):
    """A chain on one document, and a resume from each of its records, leaving the chain as it is.  Then the one that goes on writing, twice."""
    w = World(engine, 'records', main=main, extra_args=extra_args, in_place=in_place)
    w.env.update(env or {})
    w.settle()
    w.reset_out()
    ref_run = w.cold()
    ref = state(w.out)
    t = w.write_chain(RSTEX_PAGE_CK=every)
    bad = judged(ref, ref_run, state(w.out), t)
    # As the writer has it at the end: a record that did not fit was said to be one on its way.
    n = next((int(m.group(1)) for line in said(t.stderr) for m in [re.match(r'pages: (\d+) records, \d+ pages of', line)] if m), 0)
    stops = [line for line in said(t.stderr) if line.startswith('pages: st')]
    found = 0
    for k in range(1, n + 1):
        w.reset_out()
        p = w.run(RSTEX_RESUME=str(k), RSTEX_PAGE_READONLY='1', RSTEX_CHECKPOINT_TRACE='1', RSTEX_OUTCACHE_TRACE='1')
        d = judged(ref, ref_run, state(w.out), p)
        if used_record(p) != k:
            d.append(f'used {used_record(p)}')
        m = re.search(rb'rstex: outcache: streams (\d+) hits (\d+) ', p.stderr)
        found += int(m.group(2)) if m else 0
        if d:
            bad.append((k, d))
    for k in (n // 2, 0, n):
        w.reset_out()
        p = w.run(RSTEX_RESUME=str(k), RSTEX_PAGE_CK=every, RSTEX_CHECKPOINT_TRACE='1')
        d = judged(ref, ref_run, state(w.out), p)
        if d:
            bad.append(('going on', k, d))
    print(f'records {main} {" ".join(extra_args)} {" ".join(env or [])}{" in place" if in_place else ""}: {n} records, {len(bad)} differ, {found} streams from the last PDF; {stops[:1]}')
    for b in bad[:10]:
        print('  ', b)
    w.remove()
    return not bad and n >= least and (found > 0 or n == 0 or not KEEP_PDF)


def the_records(args, engine):
    ok = records(args, engine)
    ok = records(args, engine, main='groups.tex') and ok
    ok = records(args, engine, main='errors.tex', least=4) and ok
    ok = records(args, engine, main='errors.tex', extra_args=('-interaction=nonstopmode',), least=4) and ok
    ok = records(args, engine, main='wide.tex', extra_args=('-interaction=nonstopmode',), least=3) and ok
    ok = records(args, engine, main='include.tex') and ok
    ok = records(args, engine, extra_args=('-recorder',)) and ok
    ok = records(args, engine, extra_args=('-jobname=another',)) and ok
    ok = records(args, engine, main='include.tex', in_place=True) and ok
    # Where nothing can be mapped, as on Windows.
    ok = records(args, engine, env={'RSTEX_PAGE_NO_MAP': '1'}) and ok
    # A chain that stops growing, or one whose first record does not fit: what there is stays good.
    ok = records(args, engine, env={'RSTEX_PAGE_BOUND': str(6 << 20)}, least=2) and ok
    ok = records(args, engine, env={'RSTEX_PAGE_BOUND': str(1 << 16)}, least=0) and ok
    ok = nothing_to_do(engine) and ok
    ok = a_long_document(engine) and ok
    ok = what_the_recorder_would_miss(engine) and ok
    ok = the_cache_behind_a_record(engine) and ok
    ok = what_a_chain_is_keyed_on(engine) and ok
    return ok


def what_a_chain_is_keyed_on(engine):
    """The switches that change what the image holds are in a checkpoint's key, and a chain hangs from its checkpoint.  With another of them a run restores nothing of the chain's, and gives what a cold run with that switch gives; and the chain is still there for a run without."""
    bad = []
    used = []
    for what, extra_args, env in (('no shared arguments', (), {'RSTEX_NO_SHARE': '1'}), ('no native \\pickup@font', (), {'RSTEX_NO_PF': '1'}), ('no native diagrams', ('--no-commute',), {})):
        w = World(engine, 'keyed')
        w.settle()
        w.write_chain(RSTEX_PAGE_CK='1')
        w.reset_out()
        plain = w.run(RSTEX_CHECKPOINT_TRACE='1', RSTEX_PAGE_READONLY='1')
        w.argv[1:1] = extra_args
        w.reset_out()
        ref_run = w.cold(**env)
        ref = state(w.out)
        w.reset_out()
        p = w.run(RSTEX_CHECKPOINT_TRACE='1', **env)
        d = judged(ref, ref_run, state(w.out), p)
        if b'rstex: checkpoint: restored' in p.stderr:
            d.append('restored')
        for a in extra_args:
            w.argv.remove(a)
        w.reset_out()
        q = w.run(RSTEX_CHECKPOINT_TRACE='1', RSTEX_PAGE_READONLY='1')
        used.append((used_record(plain), used_record(q)))
        if used_record(q) == 0 or used_record(q) != used_record(plain):
            d.append('the chain is gone')
        if d:
            bad.append((what, d))
        w.remove()
    print(f'what a chain is keyed on: records used before and after each other kind of run {used}, {len(bad)} differ {bad[:3]}')
    return not bad


def the_cache_behind_a_record(engine):
    """An edit near the end, and then one near the start.  The run after the first resumes late and deflates none of what comes before its record, so the index it leaves has to name that as well, or the
    run after the second deflates all the pages between its record and the first's."""
    w = World(engine, 'behind', main='behind.tex')
    body = ''.join(f'Page {k} has words of its own and {k * 7} of them.\\clearpage\n' for k in range(24))
    with open(os.path.join(w.src, 'behind.tex'), 'w') as f:
        f.write('\\documentclass{article}\n\\begin{document}\n' + body + '\\end{document}\n')

    def edit(a, b):
        with open(os.path.join(w.src, 'behind.tex')) as f:
            text = f.read()
        with open(os.path.join(w.src, 'behind.tex'), 'w') as f:
            f.write(text.replace(a, b, 1))

    def go():
        """A run from what the last one left, against a cold one, which leaves the same behind it if the two are equal: (what differs, the record used, streams, those found)."""
        q = w.run(RSTEX_PAGE_CK='2', RSTEX_CHECKPOINT_TRACE='1', RSTEX_OUTCACHE_TRACE='1')
        got = state(w.out)
        ref_run = w.cold()
        d = judged(state(w.out), ref_run, got, q)
        put(w.out, got)
        m = re.search(rb'rstex: outcache: streams (\d+) hits (\d+) ', q.stderr)
        return d, used_record(q), int(m.group(1)) if m else 0, int(m.group(2)) if m else 0

    w.settle()
    w.run(RSTEX_NO_PAGES='1')
    d, _, _, _ = go()
    edit('Page 22 has', 'Page 22 now has')
    d1, late, _, _ = go()
    edit('Page 3 has', 'Page 3 now has')
    d2, early, streams, hits = go()
    print(f'the cache behind a record: resumed at {late} and then at {early}, {hits} of {streams} streams from the last PDF, {len(d + d1 + d2)} differ {(d + d1 + d2)[:3]}')
    w.remove()
    # All but the page edited and the cross-reference stream, which holds where each now is, and a few that go with it.
    return not (d or d1 or d2) and 0 < early and late > early + 4 and hits >= streams - 6


def what_the_recorder_would_miss(engine):
    """Answering the log of a record that then does not hold makes kpathsea read its font map for the first time, a file the .fls names where that happens.  A run from an earlier record would not name it."""
    w = World(engine, 'recorder', main='late.tex', extra_args=('-recorder',))
    with open(os.path.join(w.src, 'late.tex'), 'w') as f:
        f.write('\\documentclass{article}\\begin{document}First.\\clearpage Second.\\clearpage \\font\\q=nosuchfontatall \\IfFileExists{later.tex}{Yes}{No}\\clearpage Fourth.\\clearpage Fifth.\\end{document}\n')
    w.settle()
    w.write_chain(RSTEX_PAGE_CK='1')
    with open(os.path.join(w.src, 'later.tex'), 'w') as f:
        f.write('there now\n')
    w.reset_out()
    ref_run = w.cold()
    ref = state(w.out)
    w.reset_out()
    p = w.run(RSTEX_CHECKPOINT_TRACE='1', RSTEX_PAGE_CK='1')
    d = judged(ref, ref_run, state(w.out), p)
    seen = any('kpathsea has read' in s for s in said(p.stderr))
    w.reset_out()
    p = w.run(RSTEX_CHECKPOINT_TRACE='1', RSTEX_PAGE_CK='1')
    d += judged(ref, ref_run, state(w.out), p)
    print(f'what the recorder would miss: {"went cold for it" if seen else "nothing to miss"}, {len(d)} differ {d[:3]}')
    w.remove()
    return not d


def a_long_document(engine):
    """The first chain of a long document goes by its length, which the run that wrote the preamble's checkpoint has left, and so has about 64 records.  A short one has one every 4 pages."""
    w = World(engine, 'long', main='long.tex')
    with open(os.path.join(w.src, 'long.tex'), 'w') as f:
        f.write('\\documentclass{article}\\begin{document}\\newcount\\n\\loop\\advance\\n by 1 Page \\the\\n.\\clearpage\\ifnum\\n<700 \\repeat\\end{document}\n')
    w.settle()
    w.reset_out()
    ref_run = w.cold()
    ref = state(w.out)
    w.reset_out()
    w.run()
    w.reset_out()
    t = w.run(RSTEX_CHECKPOINT_TRACE='1')
    n = next((int(m.group(1)) for line in said(t.stderr) for m in [re.match(r'pages: (\d+) records, \d+ pages of', line)] if m), 0)
    d = judged(ref, ref_run, state(w.out), t)
    w.reset_out()
    p = w.run(RSTEX_CHECKPOINT_TRACE='1')
    d += judged(ref, ref_run, state(w.out), p)
    print(f'a document of 700 pages: {n} records, the last used {used_record(p)}, {len(d)} differ')
    w.remove()
    return not d and 50 <= n <= 72 and used_record(p) == n


def nothing_to_do(engine):
    """Where there is to be no chain: a run that reaches no record makes nothing, and with a switch off, or under SyncTeX, a run gives what it gave."""
    ok = True
    for what, main, extra_args, env in (('one page', 'hello.tex', (), {}), ('the switch', 'adv.tex', (), {'RSTEX_NO_PAGES': '1'}), ('SyncTeX', 'adv.tex', ('-synctex=1',), {}),
                                        ('SyncTeX by the document, and off by the record', 'turned.tex', (), {'RSTEX_PAGE_CK': '2'})):
        w = World(engine, 'none', main=main, extra_args=extra_args)
        with open(os.path.join(w.src, 'hello.tex'), 'w') as f:
            f.write('\\documentclass{article}\\begin{document}Hello.\\end{document}\n')
        # It has its file from the first page on, and \\synctex says nothing of that at the second.
        with open(os.path.join(w.src, 'turned.tex'), 'w') as f:
            f.write('\\documentclass{article}\\synctex=1 \\begin{document}One.\\clearpage\\synctex=0 Two.\\clearpage Three.\\clearpage Four.\\clearpage Five.\\clearpage Six.\\end{document}\n')
        w.settle()
        w.reset_out()
        ref_run = w.cold()
        ref = state(w.out)
        for _ in range(3):
            w.reset_out()
            p = w.run(**env)
            d = judged(ref, ref_run, state(w.out), p)
            if d or w.chain_dir():
                ok = False
                print(f'  {what}: {d}, chain {bool(w.chain_dir())}')
        w.remove()
    print(f'no chain where there is to be none: {"as it should be" if ok else "NOT"}')
    return ok


def world_cases(where, tex):
    """(name, change, environment for the runs after it)."""
    def wr(name, data):
        def f(w):
            with open(os.path.join(w.src, name), 'wb') as g:
                g.write(data)
        return f

    def sub(name, a, b, keep_time=False):
        def f(w):
            p = os.path.join(w.src, name)
            st = os.stat(p)
            with open(p, 'rb') as g:
                d = g.read()
            assert a in d, a
            with open(p, 'wb') as g:
                g.write(d.replace(a, b, 1))
            if keep_time:
                os.utime(p, ns=(st.st_atime_ns, st.st_mtime_ns))
        return f

    def touch(name):
        def f(w):
            t = time.time() + 86400 * 3
            os.utime(os.path.join(w.src, name), (t, t))
        return f

    def rm(name):
        return lambda w: os.unlink(os.path.join(w.src, name))

    def other_pixels(name):
        def f(w):
            p = os.path.join(w.src, name)
            st = os.stat(p)
            with open(p, 'wb') as g:
                g.write(snapshot_tool.tiny_png(8, 8, 40))
            os.utime(p, ns=(st.st_atime_ns, st.st_mtime_ns))
        return f

    def at(p, v):
        def f(w):
            path = os.path.join(w.src, 'adv.tex')
            with open(path, 'rb') as g:
                d = bytearray(g.read())
            d[p:p + 1] = v
            with open(path, 'wb') as g:
                g.write(bytes(d))
        return f

    def in_output(ext, a, b):
        """What the last run left says something else: what a cold run starts from too."""
        def f(w):
            for k in w.pre:
                if k.endswith(ext):
                    assert a in w.pre[k], ext
                    w.pre[k] = w.pre[k].replace(a, b, 1)
        return f

    nothing = lambda w: None  # noqa: E731
    cases = [
        ('the .aux says another page', in_output('.aux', b'{sec:one}{{1}{2}', b'{sec:one}{{1}{7}'), {}),
        ('the .toc says another page', in_output('.toc', b'Match and streams}{2}', b'Match and streams}{9}'), {}),
        ('a .aux at the policy spacing', in_output('.aux', b'{sec:one}{{1}{2}', b'{sec:one}{{1}{7}'), {'RSTEX_PAGE_CK': None}),
        ('an early read at the policy spacing', sub('data.txt', b'line one', b'line ONE'), {'RSTEX_PAGE_CK': None}),
        ('an early lookup at the policy spacing', wr('absent0.tex', b'NOW PRESENT\n'), {'RSTEX_PAGE_CK': None}),
        ('no change', nothing, {}),
        ('data.txt, a line never read', sub('data.txt', b'line seven', b'line SEVEN'), {}),
        ('data.txt, a line read late', sub('data.txt', b'line five', b'line FIVE'), {}),
        ('data.txt, a line read early', sub('data.txt', b'line one', b'line ONE'), {}),
        ('data.txt cut short', wr('data.txt', b'line one of the data\n'), {}),
        ('data.txt gone', rm('data.txt'), {}),
        ('noend.txt, the line goes on', sub('noend.txt', b'no end', b'no end, and more\nand more'), {}),
        ('noend.txt, an end of line comes', sub('noend.txt', b'no end', b'no end\n'), {}),
        ('data3 the same size', sub('data3.txt', b'three', b'THREE', keep_time=True), {}),
        ('data3 touched and no more', touch('data3.txt'), {}),
        ('data3 another size', sub('data3.txt', b'three', b'three and more'), {}),
        ('data2 same size and stamp', sub('data2.txt', b'two', b'TWO', keep_time=True), {}),
        ('absent.tex comes', wr('absent.tex', b'NOW PRESENT\n'), {}),
        ('shadow.tex comes', wr('shadow.tex', b'SHADOW PRESENT\n'), {}),
        ('sub.tex, the third part', sub('sub.tex', b'In sub, third', b'In SUB, third'), {}),
        ('sub.tex, the first part', sub('sub.tex', b'In sub, first', b'In SUB, first'), {}),
        ('sub.tex past its endinput', sub('sub.tex', b'never read', b'NEVER read'), {}),
        ('sub.tex grows', sub('sub.tex', b'never read.', b'never read.\nAnd never this.'), {}),
        ('alpha.png other pixels, same stamp', other_pixels('alpha.png'), {}),
        ('pic.png replaced', wr('pic.png', snapshot_tool.tiny_png(8, 8, 200)), {}),
        ('pic.pdf replaced', wr('pic.pdf', snapshot_tool.tiny_pdf(100)), {}),
        ('pic.jpg replaced', wr('pic.jpg', snapshot_tool.jpeg_with_payload_changed()), {}),
        ('pic.jpg gone', rm('pic.jpg'), {}),
        ('the clock two days on', nothing, {'SOURCE_DATE_EPOCH': '1700172800'}),
        ('the clock an hour on', nothing, {'SOURCE_DATE_EPOCH': '1700003600'}),
        ('a variable a record looked at', nothing, {'TEXMFOUTPUT': '/nonexistent'}),
        ('the last paragraph', sub('adv.tex', b'The last page.', b'The LAST page, and more words here.'), {}),
        ('a letter in the middle', sub('adv.tex', b'Expansion on.', b'Expansion ON.'), {}),
        ('a whole new page near the start', sub('adv.tex', b'\\section{Match and streams}', b'New words.\\clearpage\\section{Match and streams}'), {}),
        ('the preamble', sub('adv.tex', b'\\newread\\rb', b'\\newread\\rb\\newcount\\qq'), {}),
    ]
    # Exactly at the place: the byte a record has not read, and the last it has.
    for rec, page, pos, path in where:
        if path.endswith('adv.tex') and 0 < pos < len(tex) and rec % 4 == 0:
            cases.append((f'record {rec}: the byte at its place', at(pos, b'%'), {}))
            cases.append((f'record {rec}: the byte before it', at(pos - 1, b'%'), {}))
    return cases


def one_world_case(engine, name, change, env, main='adv.tex'):
    w = World(engine, 'world', main=main)
    try:
        w.settle()
        env = dict(env)
        spacing = {} if env.pop('RSTEX_PAGE_CK', '1') is None else {'RSTEX_PAGE_CK': '1'}
        w.write_chain(**spacing)
        chain = state(w.ck_dir())
        change(w)
        w.reset_out()
        ref_run = w.cold(**env)
        ref = state(w.out)
        w.reset_out()
        p = w.run(RSTEX_CHECKPOINT_TRACE='1', **spacing, **env)
        d = judged(ref, ref_run, state(w.out), p)
        used = used_record(p)
        # And once more from what that one left: it has cut the chain and gone on.
        w.reset_out()
        p = w.run(RSTEX_CHECKPOINT_TRACE='1', **spacing, **env)
        d2 = judged(ref, ref_run, state(w.out), p)
        # The chain as it was, with the world as it now is, and no writing.
        put(w.ck_dir(), chain)
        w.reset_out()
        p = w.run(RSTEX_PAGE_READONLY='1', **env)
        d3 = judged(ref, ref_run, state(w.out), p)
        # The negative control: the last record, whatever the log says.  A case that this gets right as well proves nothing about the log.
        put(w.ck_dir(), chain)
        w.reset_out()
        p = w.run(RSTEX_PAGE_READONLY='1', RSTEX_RESUME_FORCE='1', **env)
        forced = bool(judged(ref, ref_run, state(w.out), p))
        return name, used, d + d2 + d3, forced
    finally:
        w.remove()


def world(args, engine):
    w = World(engine, 'world-where')
    w.settle()
    t = w.write_chain(RSTEX_PAGE_CK='1')
    where = record_lines(t)
    with open(os.path.join(w.src, 'adv.tex'), 'rb') as f:
        tex = f.read()
    w.remove()
    cases = world_cases(where, tex)
    def in_chapter(name, a, b):
        def f(w):
            p = os.path.join(w.src, name)
            with open(p, 'rb') as g:
                d = g.read()
            with open(p, 'wb') as g:
                g.write(d.replace(a, b, 1))
        return f
    for name, change in (('a word in the last chapter', in_chapter('ch3.tex', b'with no harm', b'with no harm at all')),
                         ('a word in the first chapter', in_chapter('ch1.tex', b'with no harm', b'with no harm at all')),
                         ('a word at the end of the second', in_chapter('ch2.tex', b'{Five.}', b'{Five and a half.}')),
                         ('a label changes', in_chapter('ch2.tex', b'ch:2', b'ch:two'))):
        cases.append((name, change, {}, 'include.tex'))
    bad = 0
    stale = 0
    with cf.ThreadPoolExecutor(args.jobs) as ex:
        for name, used, d, forced in ex.map(lambda c: one_world_case(engine, *c), cases):
            print(f'  {name:<42} used {used:<4} {"same" if not d else "DIFFERS " + str(d)}{"" if forced else ", and a stale record would pass"}', flush=True)
            bad += bool(d)
            stale += forced
    print(f'world: {len(cases)} changes, {bad} differ, {len(where)} records in the chain; a stale record forced on differs in {stale} of them')
    return bad == 0 and stale >= len(cases) // 2


def clock(args, engine):
    """A chain written under the real clock and resumed a minute later, against a cold run, with only the clock masked: the log's first line, and the dates and the id in the PDF, which nothing is compressed to hide."""
    w = World(engine, 'clock', main='clock.tex')
    with open(os.path.join(w.src, 'clock.tex'), 'w') as f:
        f.write('\\pdfobjcompresslevel=0 \\pdfcompresslevel=0 \\input{adv}\n')
    for k in ('SOURCE_DATE_EPOCH', 'FORCE_SOURCE_DATE'):
        w.env.pop(k, None)
    w.settle()
    w.write_chain(RSTEX_PAGE_CK='1')
    time.sleep(args.wait)

    def mask(st):
        res = {}
        for k, v in st.items():
            v = re.sub(rb'/(CreationDate|ModDate) \(D:[^)]*\)', b'/DATE', v)
            v = re.sub(rb'/ID \[<[0-9A-F]+> <[0-9A-F]+>\]', b'/ID', v)
            if k.endswith('.log'):
                v = re.sub(rb'\d+ [A-Z]{3} \d{4} \d\d:\d\d', b'DATE', v)
            res[k] = v
        return res
    w.reset_out()
    w.cold()
    ref = mask(state(w.out))
    bad, used = [], []
    for setting in ('', '5', '1'):
        extra = {'RSTEX_RESUME': setting} if setting else {}
        w.reset_out()
        p = w.run(RSTEX_CHECKPOINT_TRACE='1', RSTEX_PAGE_READONLY='1', **extra)
        used.append(used_record(p))
        d = differs(ref, mask(state(w.out)))
        if d:
            bad.append((setting, d))
    print(f'clock: under the real clock, {args.wait} s later, records used {used}, {len(bad)} differ')
    for b in bad:
        print('  ', b)
    w.remove()
    return not bad and used[0] > 0


def damage(args, engine):
    """A flipped byte, or a file cut short, anywhere in a chain: the run sees it, or it did not matter, and the bytes are the cold run's either way.  And a record used against the log, to show that a comparison can fail at all."""
    w = World(engine, 'damage')
    w.settle()
    w.reset_out()
    ref_run = w.cold()
    ref = state(w.out)
    w.write_chain(RSTEX_PAGE_CK='1')
    chain = state(w.ck_dir())
    pg = os.path.relpath(w.chain_dir(), w.ck_dir())
    rnd = random.Random(args.seed)
    names = [k for k in chain if k.startswith(pg + '/') and not k.endswith('/lock') and chain[k]]
    bad, seen, trials = [], 0, 0
    for name in names:
        size = len(chain[name])
        for how in ['flip'] * args.flips + ['cut', 'empty']:
            trials += 1
            data = bytearray(chain[name])
            at = rnd.randrange(size)
            if how == 'flip':
                data[at] ^= 1 << rnd.randrange(8)
            elif how == 'cut':
                data = data[:at]
            else:
                data = bytearray()
            put(w.ck_dir(), chain)
            with open(os.path.join(w.ck_dir(), name), 'wb') as f:
                f.write(data)
            w.reset_out()
            p = w.run(RSTEX_CHECKPOINT_TRACE='1')
            d = judged(ref, ref_run, state(w.out), p)
            seen += any('damaged' in s for s in said(p.stderr))
            # And the run after it is right too: a damaged chain does not stay.
            w.reset_out()
            p = w.run()
            d += judged(ref, ref_run, state(w.out), p)
            if d:
                bad.append((os.path.basename(name), how, at, d))
    print(f'damage: {trials} trials over {len(names)} files of the chain, {seen} seen as damage, {len(bad)} differ')
    for b in bad[:12]:
        print('  ', b)
    # The negative control: an edit before every record, and the last record used all the same.  It must differ, or none of the above can.
    put(w.ck_dir(), chain)
    p = os.path.join(w.src, 'adv.tex')
    with open(p, 'rb') as f:
        text = f.read()
    with open(p, 'wb') as f:
        f.write(text.replace(b'Red starts.', b'Blue starts.', 1))
    w.reset_out()
    ref_run2 = w.cold()
    ref2 = state(w.out)
    w.reset_out()
    p2 = w.run(RSTEX_RESUME_FORCE='1', RSTEX_PAGE_READONLY='1')
    control = bool(judged(ref2, ref_run2, state(w.out), p2))
    print(f'negative control, a stale record forced on: {"differs, as it must" if control else "SAME, so nothing here can fail"}')
    w.remove()
    return not bad and control and strange_chains(engine)


def strange_chains(engine):
    """What is there is not what a run left: a file of the chain that is a link, a directory or open to others, and a chain that is not a directory.  A cache that is no one else's only meets these by accident, and a run goes without."""
    w = World(engine, 'strange')
    w.settle()
    w.reset_out()
    ref_run = w.cold()
    ref = state(w.out)
    w.write_chain(RSTEX_PAGE_CK='1')
    pg = w.chain_dir()
    elsewhere = w.root + '/elsewhere'
    with open(elsewhere, 'wb') as f:
        f.write(b'not a chain')

    def link(name):
        def f():
            os.unlink(os.path.join(pg, name))
            os.symlink(elsewhere, os.path.join(pg, name))
        return f

    def directory(name):
        def f():
            os.unlink(os.path.join(pg, name))
            os.mkdir(os.path.join(pg, name))
        return f

    def others(name):
        return lambda: os.chmod(os.path.join(pg, name), 0o666)

    def a_file():
        shutil.rmtree(pg)
        with open(pg, 'wb') as f:
            f.write(b'not a directory')

    cases = [('the index a link', link('f0')), ('the image a link', link('f1')), ('the lock a link', link('lock')), ('the PDF a link', link('f2')),
             ('the index a directory', directory('f0')), ('the image a directory', directory('f1')), ('the index open to others', others('f0')), ('the image open to others', others('f1')),
             ('the chain a file', a_file)]
    chain = state(w.ck_dir())
    bad = []
    for name, change in cases:
        shutil.rmtree(w.ck_dir())
        put(w.ck_dir(), chain)
        change()
        for k in range(2):
            w.reset_out()
            p = w.run()
            d = judged(ref, ref_run, state(w.out), p)
            if d:
                bad.append((name, k, d))
    with open(elsewhere, 'rb') as f:
        if f.read() != b'not a chain':
            bad.append('a link was written through')
    print(f'strange chains: {len(cases)} cases, {len(bad)} differ')
    for b in bad[:10]:
        print('  ', b)
    w.remove()
    return not bad


def corpus_world(engine, doc_id, tag):
    """A public or private document seen through links, so that it can be edited: `(doc, argv, cwd, env, out, src, root, side files to start from)`."""
    doc = proflib.resolve_docs([doc_id])[0]
    doc.prepare()
    root = fresh(tag)
    src = root + '/src'
    for r, dirs, names in os.walk(doc.entry['cwd']):
        dirs[:] = [d for d in dirs if d != '.git']
        d = os.path.normpath(os.path.join(src, os.path.relpath(r, doc.entry['cwd'])))
        os.makedirs(d, exist_ok=True)
        for n in names:
            os.symlink(os.path.join(r, n), os.path.join(d, n))
    d2 = proflib.Doc(doc.name, dict(doc.entry, cwd=src))
    d2.meta = doc.meta
    out = root + '/o'
    os.makedirs(out)
    proflib.corpus().restore_state(doc.meta, out)
    pre = state(out)
    argv, cwd, _ = engine.command(d2, out)
    env = proflib.corpus().entry_env(doc.entry)
    env.update(RSTEX_CHECKPOINT_POLICY='always', XDG_CACHE_HOME=root + '/c')
    return doc, argv, cwd, env, out, src, root, pre


def run_plain(argv, cwd, env):
    p = subprocess.run(argv, cwd=cwd, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    return p


def kill(args, engine):
    """SIGKILL in the middle of a run, as an editor that cancels would do it, some of them right after an edit, then an ordinary run.  It must still equal a cold one."""
    if args.doc:
        doc, argv, cwd, env, out, src, root, pre = corpus_world(engine, args.doc, 'kill')
    else:
        w = World(engine, 'kill')
        w.settle()
        argv, cwd, env, out, src, root, pre = w.argv, w.src, w.env, w.out, w.src, w.root, w.pre
    rnd = random.Random(args.seed)
    put(out, pre)
    run_plain(argv, cwd, dict(env, RSTEX_NO_PAGES='1'))
    put(out, pre)
    t0 = time.time()
    run_plain(argv, cwd, env)
    T = time.time() - t0
    put(out, pre)
    t = run_plain(argv, cwd, dict(env, RSTEX_RESUME='0', RSTEX_CHECKPOINT_TRACE='1', RSTEX_PAGE_INPUTS='1'))
    where = record_lines(t)
    bad, used = [], []
    edits = 0
    for trial in range(args.trials):
        # An edit now and then, after a random record, so that what a killed run was doing is cutting and appending.
        if trial % 2 and where:
            rec = rnd.choice(where)
            full = rec[3] if os.path.isabs(rec[3]) else os.path.join(cwd, rec[3])
            if os.path.normpath(full).startswith(src):
                with open(full, 'rb') as f:
                    data = f.read()
                edited = edit_bytes(data, min(rec[2], len(data)), 'word')
                if edited:
                    if os.path.islink(full):
                        os.unlink(full)
                    with open(full, 'wb') as f:
                        f.write(edited)
                    edits += 1
        for _ in range(rnd.choice([1, 1, 2, 3])):
            put_out(out, pre)
            p = subprocess.Popen(argv, cwd=cwd, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            time.sleep(rnd.random() * T * 1.1)
            p.send_signal(signal.SIGKILL)
            p.wait()
        put(out, pre)
        ref_run = run_plain(argv, cwd, dict(env, RSTEX_NO_SNAPSHOT='1'))
        ref = state(out)
        put_out(out, pre)
        r = run_plain(argv, cwd, dict(env, RSTEX_CHECKPOINT_TRACE='1'))
        d = judged(ref, ref_run, state(out), r)
        used.append(used_record(r))
        if d:
            bad.append((trial, d))
    print(f'kill {args.doc or "adv.tex"}: {args.trials} trials, {edits} edits, {len(bad)} differ; records used {used}')
    for b in bad[:10]:
        print('  ', b)
    shutil.rmtree(root, ignore_errors=True)
    return not bad


def together(args, engine):
    """Two runs at once on one chain, more than once.  One goes without, and neither leaves a chain that a later run cannot use."""
    w = World(engine, 'together')
    w.settle()
    w.reset_out()
    ref_run = w.cold()
    ref = state(w.out)
    w.write_chain(RSTEX_PAGE_CK='1')
    p = os.path.join(w.src, 'adv.tex')
    bad = []
    with open(p, 'rb') as f:
        text = f.read()
    for k in range(args.trials):
        w.reset_out()
        ps = [subprocess.Popen(w.argv, cwd=w.src, env=dict(w.env, RSTEX_PAGE_CK='1'), stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL) for _ in range(3)]
        for q in ps:
            q.wait()
            q.stdout.close()
        # What two runs do to one log is their affair, as it is for pdflatex.  The PDF is published whole, and the next run has a chain to judge.
        w.reset_out()
        r = w.run(RSTEX_CHECKPOINT_TRACE='1')
        d = judged(ref, ref_run, state(w.out), r)
        if d:
            bad.append((k, used_record(r), d))
    print(f'together: {args.trials} rounds of three runs at once, {len(bad)} differ afterwards')
    for b in bad[:10]:
        print('  ', b)
    w.remove()
    return not bad


def edit_bytes(data, pos, kind):
    at = pos
    n = len(data)
    seen = 0
    while at < n and seen < 20000:
        e = data.find(b'\n', at)
        if e < 0:
            e = n
        line = data[at:e]
        if PROSE.match(line) and at > 0 and data[at - 1:at] == b'\n':
            if kind == 'letter':
                k = line.find(b'e', 5)
                if k >= 0:
                    return data[:at + k] + b'a' + data[at + k + 1:]
            elif kind == 'word':
                k = line.find(b' ', 5)
                if k >= 0:
                    return data[:at + k] + b' indeed' + data[at + k:]
            elif kind == 'para':
                return data[:at] + PARA + data[at:]
        at = e + 1
        seen += 1
    return None


def counted(argv, cwd, env):
    r = proflib.run_counted(argv, cwd, env)
    c = r['counters']
    return {'instr': c['instructions']['value'] / 1e6, 'cycles': c['cycles']['value'] / 1e6, 'faults': c['page-faults']['value'], 'cpu': r['cpu_ms'], 'wall': r['wall_ms']}


def session(args, engine):
    """A sitting at the editor: edits placed uniformly over the records, each kept, each run to the fixed point as latexmk would run it, with page records against the preamble's checkpoint alone.
    The two must agree at every step, and the numbers say what it was worth."""
    doc, argv, cwd, env, out, src, root, base0 = corpus_world(engine, args.doc, 'session')
    caches = {'B': root + '/cB', 'A': root + '/cA'}
    for c in caches.values():
        os.makedirs(c)
    extra = {'B': {'RSTEX_NO_PAGES': '1'}, 'A': {'RSTEX_PAGE_CK': str(args.every)} if args.every else {}}

    def go(which, more=None):
        return counted(argv, cwd, dict(env, XDG_CACHE_HOME=caches[which], **extra[which], **(more or {})))

    def side(st):
        return {k: v for k, v in st.items() if k.endswith(SIDE)}

    # Both start from a preamble checkpoint with its seal, and the tables of kpathsea in the cache.
    for which in caches:
        put(out, base0)
        run_plain(argv, cwd, dict(env, XDG_CACHE_HOME=caches[which], RSTEX_NO_PAGES='1'))
    time.sleep(2.2)
    for which in caches:
        put(out, base0)
        run_plain(argv, cwd, dict(env, XDG_CACHE_HOME=caches[which], RSTEX_NO_PAGES='1'))
    put(out, base0)
    go('B')
    stB = state(out)
    put(out, base0)
    wr = go('A', {'RSTEX_CHECKPOINT_TRACE': '1', 'RSTEX_PAGE_INPUTS': '1'})
    put(out, base0)
    t = run_plain(argv, cwd, dict(env, XDG_CACHE_HOME=caches['A'], **extra['A'], RSTEX_CHECKPOINT_TRACE='1', RSTEX_PAGE_INPUTS='1', RSTEX_RESUME='0'))
    where = record_lines(t)
    stA = state(out)
    print(f'{doc.id}: the run that writes a chain: {wr["instr"]:.0f} Minstr, {wr["faults"]} faults; {len(where)} records')
    rnd = random.Random(args.seed)
    totals = {'A': [0, 0, 0, 0], 'B': [0, 0, 0, 0]}
    current = {}
    bad = 0
    mix = args.mix.split(',')
    for step in range(args.steps):
        kind = rnd.choice(mix)
        frac = rnd.random()
        k0 = int(frac * len(where))
        edited = None
        for k in list(range(k0, len(where))) + list(range(k0 - 1, -1, -1)):
            _, page, pos, path = where[k]
            full = os.path.join(src, path) if not os.path.isabs(path) else path
            if not os.path.normpath(full).startswith(src):
                continue
            with open(full, 'rb') as f:
                data = current.get(full) or f.read()
            edited = edit_bytes(data, min(pos, len(data)), kind)
            if edited:
                break
        if not edited:
            print('  no place to edit')
            continue
        current[full] = edited
        if os.path.islink(full):
            os.unlink(full)
        with open(full, 'wb') as f:
            f.write(edited)
        res = {}
        for which, st in (('B', stB), ('A', stA)):
            put(out, st)
            passes = []
            for _ in range(5):
                before = side(state(out))
                passes.append(go(which))
                after = state(out)
                if side(after) == before:
                    break
            res[which] = (passes, after)
        stB, stA = res['B'][1], res['A'][1]
        d = differs(stB, stA)
        bad += bool(d)
        for name in stB:
            if name.endswith('.log') and stB[name] != stA.get(name, b''):
                print('    log:', where_logs_differ(stB[name], stA.get(name, b'')))
        pa, pb = res['A'][0], res['B'][0]
        for which, ps in (('A', pa), ('B', pb)):
            totals[which][0] += ps[0]['instr']
            totals[which][1] += sum(x['instr'] for x in ps)
            totals[which][2] += ps[0]['cpu']
            totals[which][3] += sum(x['cpu'] for x in ps)
        print(f'  {step:>2} {kind:<7} at {frac:4.2f} page {page:>4} passes {len(pb)}/{len(pa)}  first pass Minstr {pb[0]["instr"]:>8.0f} -> {pa[0]["instr"]:>8.0f} ({pa[0]["instr"] / pb[0]["instr"]:.2f})'
              f'  to the fixed point {sum(x["instr"] for x in pb):>8.0f} -> {sum(x["instr"] for x in pa):>8.0f} ({sum(x["instr"] for x in pa) / sum(x["instr"] for x in pb):.2f})'
              f'  cpu first {pb[0]["cpu"]:>6.0f} -> {pa[0]["cpu"]:>6.0f}  all {sum(x["cpu"] for x in pb):>6.0f} -> {sum(x["cpu"] for x in pa):>6.0f}  {"same" if not d else "DIFFERS " + str(d)}', flush=True)
    A, B = totals['A'], totals['B']
    if B[0]:
        print(f'totals: first pass instr {A[0] / B[0]:.3f}, to the fixed point {A[1] / B[1]:.3f}; cpu first pass {A[2] / B[2]:.3f}, to the fixed point {A[3] / B[3]:.3f}; {bad} of {args.steps} steps differ')
    sizes = sum(len(v) for v in state(caches['A'] + '/rstex/checkpoints').values())
    print(f'the cache with a chain: {sizes / 1e6:.1f} MB')
    shutil.rmtree(root, ignore_errors=True)
    return bad == 0


def verify_one(engine, doc_id, every):
    """The state itself: a resumed run's sums at each later record against what the writer's were."""
    doc, argv, cwd, env, out, src, root, pre = corpus_world(engine, doc_id, 'verify')
    try:
        more = {'RSTEX_PAGE_CK': str(every)} if every else {}
        put(out, pre)
        ref_run = run_plain(argv, cwd, dict(env, RSTEX_NO_SNAPSHOT='1'))
        ref = state(out)
        put(out, pre)
        run_plain(argv, cwd, dict(env, RSTEX_NO_PAGES='1'))
        put(out, pre)
        t = run_plain(argv, cwd, dict(env, RSTEX_PAGE_HASH='1', RSTEX_DEFLATE_WORKERS='0', RSTEX_CHECKPOINT_TRACE='1', RSTEX_PAGE_INPUTS='1', **more))
        # Under --keep-pdf that chain has deflated all it made, and each run below finds it in the PDF: so this also says that a stream from there leaves the state as a stream deflated does.
        n = len(record_lines(t))
        if n == 0:
            return doc.id, 0, 0, 0, []
        checked, differ, bad = 0, 0, []
        for k in sorted({1, max(1, n // 2), max(1, n - 1)}):
            put_out(out, pre)
            p = run_plain(argv, cwd, dict(env, RSTEX_RESUME=str(k), RSTEX_PAGE_VERIFY='1', RSTEX_DEFLATE_WORKERS='0', RSTEX_CHECKPOINT_TRACE='1'))
            d = judged(ref, ref_run, state(out), p)
            if d:
                bad.append((k, d))
            for line in said(p.stderr):
                m = re.match(r'verify: (\d+) pages checked, (\d+) differ', line)
                if m:
                    checked += int(m.group(1))
                    differ += int(m.group(2))
                elif line.startswith('verify: page') and len(bad) < 3:
                    bad.append((k, line))
        return doc.id, n, checked, differ, bad
    finally:
        shutil.rmtree(root, ignore_errors=True)


def verify(args, engine):
    bad = 0
    total = 0
    docs = args.docs.split(',')
    with cf.ThreadPoolExecutor(args.jobs) as ex:
        for doc_id, n, checked, differ, b in ex.map(lambda d: verify_one(engine, d, args.every), docs):
            print(f'  {doc_id}: {n} records, {checked} states checked, {differ} differ{" " + str(b) if b else ""}', flush=True)
            bad += differ + len(b)
            total += checked
    print(f'verify: {len(docs)} documents, {total} states checked, {bad} differ')
    return bad == 0 and total > 0


def reclaim(args, engine):
    """On a build with the feature tight_heap, whose heap this document runs out of a dozen times.  A record lies between one reclaiming and the next, and a run resumed there has to reclaim where the writer did: the state is held
    against the writer's at each later record, not only the output.  On any other build this reclaims nothing and proves nothing."""
    w = World(engine, 'reclaim', main='reclaim.tex')
    w.settle()
    w.reset_out()
    ref_run = w.cold()
    ref = state(w.out)
    t = w.write_chain(RSTEX_PAGE_CK='1', RSTEX_PAGE_HASH='1', RSTEX_DEFLATE_WORKERS='0')
    bad = judged(ref, ref_run, state(w.out), t)
    n = len(record_lines(t))
    checked, differ = 0, 0
    for k in sorted({1, n // 4, n // 2, 3 * n // 4, n - 1} - {0}):
        w.reset_out()
        p = w.run(RSTEX_RESUME=str(k), RSTEX_PAGE_VERIFY='1', RSTEX_DEFLATE_WORKERS='0', RSTEX_CHECKPOINT_TRACE='1')
        d = judged(ref, ref_run, state(w.out), p)
        if d:
            bad.append((k, d))
        for line in said(p.stderr):
            m = re.match(r'verify: (\d+) pages checked, (\d+) differ', line)
            if m:
                checked += int(m.group(1))
                differ += int(m.group(2))
            elif line.startswith('verify: page') and len(bad) < 3:
                bad.append((k, line))
    print(f'reclaim: exit {ref_run.returncode}, {n} records, {checked} states checked, {differ} differ, {len(bad)} others {bad[:3]}')
    w.remove()
    return ref_run.returncode == 0 and not bad and not differ and checked > 0


def gate(args, engine):
    results = []
    for name, f in (('records', the_records), ('world', world), ('damage', damage), ('kill', kill), ('together', together), ('clock', clock)):
        ok = f(args, engine)
        print(f'{name}: {"pass" if ok else "FAIL"}', flush=True)
        results.append((name, ok))
    args.docs = args.docs or 'ladder/17-class-scrbook,arxiv/math.AG/1912.07119v3,arxiv/math.AG/1501.07625v3,arxiv/stat.ML/2002.06694v2'
    ok = verify(args, engine)
    print(f'verify: {"pass" if ok else "FAIL"}', flush=True)
    results.append(('verify', ok))
    # Edits one after another, on a real document: it was this that found a chain blind to what came before its first record.
    args.doc, args.steps = args.doc or 'arxiv/math.AG/1912.07119v3', 8
    ok = session(args, engine)
    print(f'session: {"pass" if ok else "FAIL"}', flush=True)
    results.append(('session', ok))
    failed = [n for n, ok in results if not ok]
    print(f'{"PASS" if not failed else "FAIL " + " ".join(failed)} page records')
    return not failed


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('what', choices=['gate', 'records', 'world', 'damage', 'kill', 'together', 'clock', 'session', 'verify', 'reclaim'])
    ap.add_argument('--engine', required=True)
    ap.add_argument('-j', '--jobs', type=int, default=4)
    ap.add_argument('--doc')
    ap.add_argument('--docs')
    ap.add_argument('--steps', type=int, default=14)
    ap.add_argument('--every', type=int, default=0, help='a record every so many pages; by default what the program does by itself')
    ap.add_argument('--seed', type=int, default=5)
    ap.add_argument('--mix', default='letter,word,word,para')
    ap.add_argument('--trials', type=int, default=20)
    ap.add_argument('--flips', type=int, default=4)
    ap.add_argument('--wait', type=int, default=61)
    ap.add_argument('--keep-pdf', action='store_true', help='a run finds the PDF the last one left')
    args = ap.parse_args()
    global KEEP_PDF
    KEEP_PDF = args.keep_pdf
    os.makedirs(ROOT, exist_ok=True)
    engine = proflib.Engine('new', args.engine)
    f = {'gate': gate, 'records': the_records, 'world': world, 'damage': damage, 'kill': kill, 'together': together, 'clock': clock, 'session': session, 'verify': verify, 'reclaim': reclaim}[args.what]
    return 0 if f(args, engine) else 1


if __name__ == '__main__':
    sys.exit(main())
