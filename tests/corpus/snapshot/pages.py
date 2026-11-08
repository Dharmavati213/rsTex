#!/usr/bin/env -S python3 -I
r"""Are page records sound?  crates/engine/src/checkpoint/pages.rs.  A mistake there is silently wrong output after an edit, so all of this asks one question: does a run that resumes give what a cold run gives?

    python3 -I tests/corpus/snapshot/pages.py gate --engine BIN [-j N]              all that follows on a document made here, and on public documents
    python3 -I tests/corpus/snapshot/pages.py records --engine BIN                  a chain on the document made here, and a resume from each record
    python3 -I tests/corpus/snapshot/pages.py world --engine BIN [-j N]             the world changes between runs, in every way that document can see
    python3 -I tests/corpus/snapshot/pages.py damage --engine BIN                   bytes flipped and files cut in a chain
    python3 -I tests/corpus/snapshot/pages.py kill --engine BIN [--doc ID] [--trials N]    SIGKILL at a random moment, also while a resumed run cuts and appends
    python3 -I tests/corpus/snapshot/pages.py together --engine BIN                 runs at once on one chain
    python3 -I tests/corpus/snapshot/pages.py clock --engine BIN [--wait S]         a chain under the real clock, used a minute later
    python3 -I tests/corpus/snapshot/pages.py session --engine BIN --doc ID [--steps N] [--every N] [--seed S] [--mix letter,word,para] [--flags "..."] [--where 0.9-1.0]    a sitting at the editor, to the fixed point
    python3 -I tests/corpus/snapshot/pages.py verify --engine BIN --docs ID,ID      the state itself, against each later record
    python3 -I tests/corpus/snapshot/pages.py reclaim --engine BIN                  on a build with the feature tight_heap: a heap that runs out of bump space, resumed
    python3 -I tests/corpus/snapshot/pages.py restart --engine BIN                  an error in a cell starts the job again with the native path off, from a record and from a checkpoint: as a run with the path off

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
# SyncTeX by the document.  It has its file from the first page on, and \\synctex says nothing of that at the second.
TURNED = '\\documentclass{article}\\synctex=1 \\begin{document}One.\\clearpage\\synctex=0 Two.\\clearpage Three.\\clearpage\\synctex=1 Four.\\clearpage Five.\\clearpage Six.\\end{document}\n'
# And asked for too late: the first sheet found it at 0, which turns it off for good.  No record finds a file.
LATE_SYNCTEX = '\\documentclass{article}\\begin{document}One.\\clearpage\\synctex=1 Two.\\clearpage Three.\\clearpage Four.\\end{document}\n'
# What can stand around the end of \document, where a run from the preamble takes a record of its own at the fetch of \ignorespaces.  (name, its text, whether that record is to be there.)
THREE_PAGES = r'First words of it.\clearpage Second page.\clearpage Third page.\clearpage Fourth page.' + '\n\\end{document}\n'
DOCUMENT_ENDS = [
    ('end-spaces.tex', '\\documentclass{article}\n\\begin{document}     \n\n   ' + THREE_PAGES, True),
    ('end-same-line.tex', '\\documentclass{article}\\begin{document}' + THREE_PAGES, True),
    ('end-tracing.tex', '\\documentclass{article}\n\\tracingcommands=2 \\tracingmacros=1 \\tracingonline=1 \n\\begin{document} \\tracingcommands=0 \\tracingmacros=0 ' + THREE_PAGES, True),
    # Not in vertical mode, where a space would count.
    ('end-hmode.tex', '\\documentclass{article}\n\\AtBeginDocument{\\leavevmode Before.}\n\\begin{document}     ' + THREE_PAGES, True),
    # A class that wraps it is still in that body, and what comes after the word is not read yet.
    ('end-wrapped.tex', '\\documentclass{article}\n\\let\\origdoc\\document \\def\\document{\\origdoc\\ignorespaces Wrapped. }\n\\begin{document}\n' + THREE_PAGES, True),
    # The word in other mouths, before the one that counts.
    ('end-hook-word.tex', '\\documentclass{article}\n\\def\\foo{\\ignorespaces}\n\\AtBeginDocument{\\ignorespaces\\ignorespaces \\foo \\edef\\bar{\\noexpand\\ignorespaces}\\let\\baz\\ignorespaces \\bar\\baz}\n\\begin{document}\n' + THREE_PAGES, True),
    # Taken away, for good and for a while, the second leaving an armed copy on the save stack.
    ('end-gone-in-preamble.tex', '\\documentclass{article}\n\\let\\ignorespaces\\relax\n\\begin{document}\n' + THREE_PAGES, False),
    ('end-gone-in-hook.tex', '\\documentclass{article}\n\\AtBeginDocument{\\let\\ignorespaces\\relax}\n\\begin{document}\n' + THREE_PAGES, False),
    ('end-group.tex', '\\documentclass{article}\n\\AtBeginDocument{\\begingroup\\let\\ignorespaces\\relax\\endgroup}\n\\begin{document}\n' + THREE_PAGES, True),
    ('end-left-in-group.tex', '\\documentclass{article}\n\\AtBeginDocument{\\begingroup\\let\\ignorespaces\\relax\\aftergroup\\relax}\n\\begin{document}\\endgroup\n' + THREE_PAGES, False),
    # A page, and an object, before it.
    ('end-hook-page.tex', '\\documentclass{article}\n\\AtBeginDocument{\\shipout\\hbox{A page from a hook.}}\n\\begin{document}\n' + THREE_PAGES, False),
    ('end-hook-object.tex', '\\documentclass{article}\n\\AtBeginDocument{\\immediate\\pdfobj stream {An object.}\\pdfximage{pic.png}\\setbox0\\hbox{A form.}\\immediate\\pdfxform0 }\n\\begin{document}\n\\pdfrefximage\\pdflastximage\\ ' + THREE_PAGES, True),
    # Files open across it.
    ('end-streams.tex', '\\documentclass{article}\n\\newwrite\\wa \\newread\\ra\n\\AtBeginDocument{\\immediate\\openout\\wa=end-side.txt \\immediate\\write\\wa{Written in a hook.}\\openin\\ra=data.txt \\read\\ra to\\linea}\n\\begin{document}\n\\linea\\ \\immediate\\write\\wa{And after.}\\immediate\\closeout\\wa \\read\\ra to\\lineb \\lineb\\ ' + THREE_PAGES, True),
    ('end-aux.tex', '\\documentclass{article}\n\\makeatletter\n\\AtBeginDocument{\\immediate\\write\\@auxout{\\string\\newlabel{hook}{{1}{1}}}\\label{inhook}}\n\\makeatother\n\\begin{document}\n\\ref{hook}\\ref{inhook}\\ ' + THREE_PAGES, True),
    ('end-nofiles.tex', '\\documentclass{article}\n\\nofiles\n\\begin{document}\n' + THREE_PAGES, True),
    # The document's body in a file of its own, opened just past the record, or the line that holds \begin{document} there.
    ('end-input-next.tex', '\\documentclass{article}\n\\begin{document}\n\\input{end-body}\n\\end{document}\n', True),
    ('end-input-before.tex', '\\documentclass{article}\n\\input{end-head}\n', True),
    ('end-endinput.tex', '\\documentclass{article}\n\\begin{document}\\input{end-tail}\\endinput\nNever read.\n', True),
    # Errors in the hook, and text where the date is read.
    ('end-error.tex', '\\documentclass{article}\n\\AtBeginDocument{\\undefinedthing}\n\\begin{document}\n' + THREE_PAGES, True),
    ('end-date.tex', '\\documentclass{article}\n\\AtBeginDocument{\\edef\\today{\\number\\year}}\n\\begin{document}\n\\today\\ ' + THREE_PAGES, True),
    # SyncTeX by the document, with its file made after the preamble.
    ('end-synctex.tex', '\\documentclass{article}\n\\AtBeginDocument{\\synctex=1 \\input{end-small}}\n\\begin{document}\n' + THREE_PAGES, True),
]
DOCUMENT_END_FILES = [('end-body.tex', 'In the body file.\\clearpage Next page there.\\clearpage Another page.\\clearpage Last page there.\n'), ('end-small.tex', 'Small.\n'),
                      ('end-tail.tex', 'In the last file.\\clearpage Next page there.\\clearpage Another page.\\clearpage Last page there.\\end{document}\n'),
                      ('end-head.tex', '\\begin{document}\n' + THREE_PAGES)]
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


# What lies above the heap's top is 0, in a run from a record, which holds the heap as far as it is live: so it must be 0 in the writer too, or a block made there later keeps what was there past its terminator, and the two states differ
# from the next record on.  A runaway's tokens go up there for the moment, in tb_show.  Nothing here would ever read them: it is the state's sum that sees it, and that sum is what says a resumed run is the writer's.
RUNAWAY = r'''\documentclass{article}
\newtoks\this
\def\mk#1{\this=\expandafter{\romannumeral\numexpr#1*1000\relax}}
\def\needsastop#1\stop{}
\begin{document}
The first page.
\clearpage
\mk{20000}\expandafter\needsastop\the\this\par
After the error.
\clearpage
\mk{15000}\edef\two{\the\this}
A page after a list of a size whose block has room to spare, on top of the heap.
\clearpage
The last page.
\end{document}
'''
# The same when tok_coalesce gives the end of the heap back to the bump space, on a heap of its full size: lists of one size up to near the end of it, all freed, and a larger one that does not fit, so that nothing can be split for it.  The record comes
# between that and the next block made up there.
COALESCE = r'''\documentclass{article}
\newtoks\this \newcount\n
\def\mk#1{\this=\expandafter{\romannumeral\numexpr#1*1000\relax}}
\begin{document}
The first page.
\clearpage
\mk{2000000}
\n=0 \loop\ifnum\n<28 \expandafter\edef\csname x\the\n\endcsname{\the\this}\advance\n1 \repeat
\n=0 \loop\ifnum\n<28 \expandafter\let\csname x\the\n\endcsname\relax \advance\n1 \repeat
\edef\three{\the\this\the\this\the\this}
The bump space has come back.
\clearpage
\edef\two{\the\this}
A page after a list of a size whose block has room to spare, on top of the heap.
\clearpage
The last page.
\end{document}
'''


# Diagrams on many pages, with references that need the last run's .aux and a .toc: a run that goes wrong in the end has to find those as the last run left them, whatever it has written since.
# Pages that each change a good part of the image, so that a chain gets long with next to nothing in the PDF.
ROOMY = r'''\documentclass{article}
\usepackage{tikz-cd}
\begin{document}
''' + ''.join(f'Page {k} of the document, with words that take up a little room, and $x^{{{k}}}+y^2=z^2$ inline.\\newcommand\\mac{"".join("abcdefghij"[int(c)] for c in str(k))}{{x}}\\clearpage\n' for k in range(120)) + r'''
\end{document}
'''
RESTART_PAGE = r'''Page %d of the document, with words of its own, and it refers to page \pageref{last} and to section \ref{last}.
\[\begin{tikzcd} A_{%d} \arrow[r,"f"] & B \arrow[d] \\ & C_{%d} \end{tikzcd}\]
\clearpage
'''
RESTART = r'''\documentclass{article}
\usepackage{tikz-cd}
\newwrite\own \immediate\openout\own=own.txt \immediate\write\own{from the preamble}\immediate\closeout\own
\begin{document}
\tableofcontents
\clearpage
\section{First}
''' + ''.join(RESTART_PAGE % (k, k, k) for k in range(1, 6)) + r'''\section{Second}
''' + ''.join(RESTART_PAGE % (k, k, k) for k in range(6, 11)) + r'''\section{Third}
''' + ''.join(RESTART_PAGE % (k, k, k) for k in range(11, 16)) + r'''\section{Last}\label{last}
The end of it.
%%EDIT%%
\end{document}
'''
# What a first diagram's cells reach by way of macros of LaTeX's, and a later one does not, moving between two records: the writer looks at it again, and a run from the first record has to.
CLOSURE = r'''\documentclass{article}
\usepackage{tikz-cd}
\begin{document}
A page of words.
\[\begin{tikzcd} \textbf{A} \arrow[r,"\textit{f}"] & \mathbf{B} \end{tikzcd}\]
\clearpage
Another page of words.
\clearpage
\renewcommand\bfdefault{b}\renewcommand\itdefault{it}\renewcommand\bfseries{\fontseries\bfdefault\selectfont}
A third page of words, with the macros of those cells made again.
\clearpage
\[\begin{tikzcd} C \arrow[r] & D \end{tikzcd}\]
\clearpage
\renewcommand\bfdefault{bx}
The last but one.
\clearpage
\[\begin{tikzcd} E \arrow[r] & \textbf{F} \end{tikzcd}\]
\clearpage
The last page.
\end{document}
'''
RESTART_EDIT = r'''\newcommand\bad{\errmessage{This is bad}}
\begin{tikzcd}
\bad A \arrow[r,"f"] & B
\end{tikzcd}
'''


def write_adv(src):
    os.makedirs(src, exist_ok=True)
    for name, text in (*[(d[0], d[1]) for d in DOCUMENT_ENDS], *DOCUMENT_END_FILES, ('adv.tex', ADV), ('sub.tex', SUB), ('groups.tex', GROUPS), ('errors.tex', ERRORS), ('wide.tex', WIDE), ('reclaim.tex', RECLAIM), ('roomy.tex', ROOMY), ('runaway.tex', RUNAWAY), ('coalesce.tex', COALESCE), ('restart.tex', RESTART), ('closure.tex', CLOSURE), ('include.tex', INCLUDE), ('turned.tex', TURNED), ('late-synctex.tex', LATE_SYNCTEX), ('ch1.tex', CHAPTER % {'n': 1}), ('ch2.tex', CHAPTER % {'n': 2}), ('ch3.tex', CHAPTER % {'n': 3}),
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
    # SyncTeX's file: each way of asking for it, and what an editor passes.
    for flags in (('-synctex=1',), ('-synctex=-1',), ('-synctex=15',), ('-synctex=1', '-recorder', '-interaction=nonstopmode', '-file-line-error')):
        ok = records(args, engine, extra_args=flags) and ok
    ok = records(args, engine, main='include.tex', extra_args=('-synctex=1',)) and ok
    ok = records(args, engine, main='errors.tex', extra_args=('-synctex=1', '-interaction=nonstopmode'), least=4) and ok
    ok = records(args, engine, main='turned.tex') and ok
    ok = records(args, engine, main='late-synctex.tex') and ok
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
    ok = the_end_of_document(engine) and ok
    ok = the_closure_file(engine) and ok
    ok = what_a_chain_is_keyed_on(engine) and ok
    ok = no_room_for_the_chain(engine) and ok
    ok = a_date_of_another_width(engine) and ok
    ok = synctex_read_in_the_body(engine) and ok
    # What the native path keeps from one diagram to the next is no part of a record, and may not show in the image.
    ok = states_held(engine, 'restart.tex') and ok
    ok = states_held(engine, 'closure.tex') and ok
    # What lies above the top of the heap: the tokens of a runaway, and what tok_coalesce gave back.
    ok = states_held(engine, 'runaway.tex', status=1) and ok
    ok = states_held(engine, 'coalesce.tex', status=None) and ok
    return ok


def the_end_of_document(engine):
    """The record at the end of \\document: what comes back from it, whether it is there where it should be, its state against the writer's at each page after it, and a word changed just past it, which only that record survives."""
    ok = True
    for main, _, wanted in DOCUMENT_ENDS:
        for extra_args in ((), ('-synctex=1', '-recorder', '-interaction=nonstopmode', '-file-line-error')):
            w = World(engine, 'docend', main=main, extra_args=extra_args)
            w.settle()
            t = w.write_chain(RSTEX_PAGE_CK='1')
            where = record_lines(t)
            first = bool(where) and where[0][1] == 0
            bad = []
            if first != wanted:
                bad.append(f'a record at the start: {first}')
            with open(os.path.join(w.src, main), 'rb') as f:
                text = f.read()
            if first and any(b'First words of it.' in line and b'begin{document}' not in line for line in text.split(b'\n')):
                with open(os.path.join(w.src, main), 'wb') as f:
                    f.write(text.replace(b'First words of it.', b'First WORDS of it, with more.', 1))
                w.reset_out()
                ref_run = w.cold()
                ref = state(w.out)
                w.reset_out()
                p = w.run(RSTEX_CHECKPOINT_TRACE='1', RSTEX_PAGE_READONLY='1')
                bad += judged(ref, ref_run, state(w.out), p)
                if used_record(p) != 1:
                    bad.append(f'used {used_record(p)} after a word on the first page')
            print(f'the end of document {main} {" ".join(extra_args)}: {len(where)} records, {"at" if first else "not at"} the start, {len(bad)} wrong {bad[:3]}', flush=True)
            ok = not bad and ok
            w.remove()
        ok = records(None, engine, main=main, least=0) and ok
        ok = records(None, engine, main=main, extra_args=('-synctex=1', '-recorder', '-interaction=nonstopmode', '-file-line-error'), least=0) and ok
        ok = states_held(engine, main, status=None) and ok
    return ok


def xxh64(data, seed):
    """As checkpoint.rs has it, to write a file that passes its sum and says something else."""
    p1, p2, p3, p4, p5 = 0x9E3779B185EBCA87, 0xC2B2AE3D27D4EB4F, 0x165667B19E3779F9, 0x85EBCA77C2B2AE63, 0x27D4EB2F165667C5
    m = (1 << 64) - 1
    rol = lambda x, r: ((x << r) | (x >> (64 - r))) & m  # noqa: E731
    rnd = lambda a, x: (rol((a + x * p2) & m, 31) * p1) & m  # noqa: E731
    n, at = len(data), 0
    if n >= 32:
        v = [(seed + p1 + p2) & m, (seed + p2) & m, seed, (seed - p1) & m]
        while at + 32 <= n:
            for k in range(4):
                v[k] = rnd(v[k], int.from_bytes(data[at:at + 8], 'little'))
                at += 8
        h = (rol(v[0], 1) + rol(v[1], 7) + rol(v[2], 12) + rol(v[3], 18)) & m
        for x in v:
            h = ((h ^ rnd(0, x)) * p1 + p4) & m
    else:
        h = (seed + p5) & m
    h = (h + n) & m
    while at + 8 <= n:
        h = (rol(h ^ rnd(0, int.from_bytes(data[at:at + 8], 'little')), 27) * p1 + p4) & m
        at += 8
    if at + 4 <= n:
        h = (rol(h ^ (int.from_bytes(data[at:at + 4], 'little') * p1) & m, 23) * p2 + p3) & m
        at += 4
    while at < n:
        h = (rol(h ^ (data[at] * p5) & m, 11) * p1) & m
        at += 1
    h ^= h >> 33
    h = (h * p2) & m
    h ^= h >> 29
    h = (h * p3) & m
    return h ^ (h >> 32)


def the_closure_file(engine):
    """What earlier runs found in the preamble's macros, in a file beside its checkpoint, takes the place of a look at each and nothing else: a chain written with no such file is held, in its state, against a run from the record at the end of
    \\document that has it, whose first diagram walks.  A file with a sum that passes and something wrong in it has to show there too, or none of this proves anything.  And a damaged one is only not used."""
    ok = True
    for main in ('closure.tex', 'restart.tex'):
        w = World(engine, 'closure', main=main)
        w.settle()
        w.reset_out()
        ref_run = w.cold()
        ref = state(w.out)
        w.reset_out()
        w.run(RSTEX_NO_PAGES='1', RSTEX_NO_CLOSURE_FILE='1')
        w.reset_out()
        t = w.run(RSTEX_NO_CLOSURE_FILE='1', RSTEX_CHECKPOINT_TRACE='1', RSTEX_PAGE_CK='1', RSTEX_PAGE_HASH='1', RSTEX_DEFLATE_WORKERS='0')
        bad = judged(ref, ref_run, state(w.out), t)
        cl = [n for n in os.listdir(w.ck_dir()) if n.endswith('.cl')]
        if cl:
            bad.append('a file with the switch off')
        # This one leaves the file, and writes nothing of a chain.
        w.reset_out()
        p = w.run(RSTEX_RESUME='0', RSTEX_PAGE_READONLY='1', RSTEX_CHECKPOINT_TRACE='1')
        bad += judged(ref, ref_run, state(w.out), p)
        cl = [os.path.join(w.ck_dir(), n) for n in os.listdir(w.ck_dir()) if n.endswith('.cl')]
        if len(cl) != 1:
            bad.append('no file was left')
            print(f'the closure file {main}: {bad}')
            w.remove()
            ok = False
            continue

        def verified():
            w.reset_out()
            p = w.run(RSTEX_RESUME='1', RSTEX_PAGE_VERIFY='1', RSTEX_DEFLATE_WORKERS='0', RSTEX_CHECKPOINT_TRACE='1')
            lines = said(p.stderr)
            taken = next((int(m.group(1)) for line in lines for m in [re.match(r'closure: (\d+) bodies from the file', line)] if m), 0)
            v = next((m for line in lines for m in [re.match(r'verify: (\d+) pages checked, (\d+) differ', line)] if m), None)
            return p, taken, int(v.group(1)) if v else 0, int(v.group(2)) if v else 0
        p, taken, checked, differ = verified()
        bad += judged(ref, ref_run, state(w.out), p)
        if not taken or not checked or differ:
            bad.append(f'{taken} from the file, {checked} states, {differ} differ')
        with open(cl[0], 'rb') as f:
            good = f.read()
        # A flipped byte anywhere in it: not used, with the same bytes.
        for at in (0, len(good) // 2, len(good) - 1):
            with open(cl[0], 'wb') as f:
                f.write(good[:at] + bytes([good[at] ^ 16]) + good[at + 1:])
            p, taken, _, differ2 = verified()
            bad += judged(ref, ref_run, state(w.out), p)
            if taken or differ2:
                bad.append(f'a flip at {at} was used')
        # The negative control: an entry that says its body mentions nothing, with its sum made good.
        body, tag = good[:-16], good[-16:-8]
        n = int.from_bytes(body[:4], 'little')
        wrong = bytearray(body)
        cut = 0
        for k in range(n):
            at = 8 + 32 * k
            a, b = int.from_bytes(body[at + 24:at + 28], 'little'), int.from_bytes(body[at + 28:at + 32], 'little')
            if b > a:
                wrong[at + 28:at + 32] = wrong[at + 24:at + 28]
                cut += 1
        with open(cl[0], 'wb') as f:
            f.write(bytes(wrong) + tag + xxh64(bytes(wrong), int.from_bytes(tag, 'little')).to_bytes(8, 'little'))
        p, taken, checked, differ3 = verified()
        if not taken or not differ3:
            bad.append(f'a wrong file with {cut} entries cut showed nothing: {taken} from it, {checked} states, {differ3} differ')
        with open(cl[0], 'wb') as f:
            f.write(good)
        print(f'the closure file {main}: {n} entries, {checked} states with it, {differ} differ, a wrong file differs in {differ3}, {len(bad)} wrong {bad[:3]}', flush=True)
        ok = not bad and ok
        w.remove()
    return ok


def no_room_for_the_chain(engine):
    """A limit on a file's size that the document's outputs stay under, as on a nearly full disk or in a sandbox: the chain stops where it is and the run goes on.  It used to go on looking at what it took to be in the chain, past the end
    of the file, and SIGBUS ended a run of a document with nothing wrong with it."""
    import resource
    w = World(engine, 'room', main='roomy.tex')
    w.settle()
    w.reset_out()
    ref_run = w.cold()
    ref = state(w.out)
    w.reset_out()
    w.run(RSTEX_NO_PAGES='1')
    ck = next(os.path.join(w.ck_dir(), n) for n in os.listdir(w.ck_dir()) if n.endswith('.ck'))
    limit = os.path.getsize(ck) + (2 << 20)

    def limited():
        resource.setrlimit(resource.RLIMIT_FSIZE, (limit, limit))
    w.reset_out()
    p = subprocess.run(w.argv, cwd=w.src, env=dict(w.env, RSTEX_PAGE_CK='1', RSTEX_CHECKPOINT_TRACE='1'), stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE, preexec_fn=limited)
    bad = judged(ref, ref_run, state(w.out), p)
    stopped = any(line.startswith('pages: stop') and 'cannot be written' in line for line in said(p.stderr))
    # What it left is good as far as it goes, for a run with room.
    w.reset_out()
    q = w.run(RSTEX_PAGE_CK='1', RSTEX_CHECKPOINT_TRACE='1')
    again = judged(ref, ref_run, state(w.out), q)
    print(f'no room for the chain: {"stopped" if stopped else "did not stop"}, {len(bad)} differ; the next run used {used_record(q)} records, {len(again)} differ {bad[:3]} {again[:3]}')
    w.remove()
    return stopped and not bad and not again and used_record(q) > 0


# A log still in its first buffer at the record a run resumes from, or past it, and a body that asks on the pages after that record how much of its log is on the disk.
DATE_WIDTH = r'''\documentclass{article}
\def\probe{\message{[\pdffilesize{\jobname.log}]}}
\def\pad#1{\count255=0 \loop\wlog{A line of the log, \the\count255, that adds up after some of them to more than a buffer holds.}\advance\count255 by 1 \ifnum\count255<#1 \repeat}
\begin{document}
Page 0.\clearpage Page 1.\clearpage Page 2.\pad{%(lines)d}\clearpage Page 3.\clearpage Page 4.\clearpage Page 5.\clearpage Page 6.\clearpage Page 7.\clearpage
Page 8 has words.\probe\clearpage Page 9.\probe\pad{80}\probe\clearpage Page 10.
\end{document}
'''


def a_date_of_another_width(engine):
    """The date in the log's first line is a digit narrower on the 4th of a month than on the 14th.  A chain is used on a day of another width than the one it was written on, or than its checkpoint's, and after its record the log has on
    the disk what a cold run of that day has there.  A narrower date used to make the chain look damaged when nothing of the log had gone to the disk, and so a cold run that wrote the chain again; and a wider one, or a log past its
    first buffer, came back with a byte more or less on the disk."""
    bad = []
    # (the checkpoint's day, the chain's, the day it is used), as SOURCE_DATE_EPOCH: 14 NOV 2023, 4 DEC, 14 DEC.
    days = (('1700000000', '1700000000', '1701728000'), ('1701728000', '1701728000', '1702592000'), ('1700000000', '1701728000', '1702592000'))
    for lines in (1, 40):
        for ck_day, chain_day, day in days:
            w = World(engine, 'datewidth', main='datewidth.tex')
            with open(os.path.join(w.src, 'datewidth.tex'), 'w') as f:
                f.write(DATE_WIDTH % {'lines': lines})
            w.env['SOURCE_DATE_EPOCH'] = ck_day
            w.settle()
            w.write_chain(RSTEX_PAGE_CK='1', SOURCE_DATE_EPOCH=chain_day)
            with open(os.path.join(w.src, 'datewidth.tex')) as f:
                text = f.read()
            with open(os.path.join(w.src, 'datewidth.tex'), 'w') as f:
                f.write(text.replace('Page 8 has', 'Page 8 now has'))
            # The record an edit there resumes from on the chain's own day, leaving the chain as it is.
            w.reset_out()
            control = used_record(w.run(RSTEX_CHECKPOINT_TRACE='1', RSTEX_PAGE_READONLY='1', SOURCE_DATE_EPOCH=chain_day))
            w.env['SOURCE_DATE_EPOCH'] = day
            w.reset_out()
            ref_run = w.cold()
            ref = state(w.out)
            w.reset_out()
            p = w.run(RSTEX_CHECKPOINT_TRACE='1', RSTEX_PAGE_CK='1')
            d = judged(ref, ref_run, state(w.out), p)
            if b'rstex: checkpoint: restored' not in p.stderr or used_record(p) != control or control < 2:
                d.append(f'resumed from record {used_record(p) if b"rstex: checkpoint: restored" in p.stderr else 0}, and {control} on the day of the chain')
            if d:
                bad.append((lines, ck_day, chain_day, day, d))
            w.remove()
    print(f'a date of another width: {len(bad)} wrong {bad[:3]}')
    return not bad


# A body that asks, on its last pages, how much of SyncTeX's working file is on the disk and what it holds.  A page's records are less than a 4 KiB buffer of text, so plain text waits in it across pages.  The pages
# before do not ask: the file is gone by the next run, and a record after such a read would not hold.
SYNCTEX_BUSY = r'''\documentclass{article}
\def\words{Some words of text on this page, and more of them, to fill a line or two of it.\par}
\def\probe{\message{[\pdffilesize{\jobname.synctex(busy)} \pdfmdfivesum file{\jobname.synctex(busy)}]}}
\begin{document}
''' + ''.join(f'Page {k} has words.{p}\\words{p}\\clearpage\n' for k, p in ((k, r'\probe' if k >= 10 else '') for k in range(13))) + '\\end{document}\n'


def synctex_read_in_the_body(engine):
    """A page record writes nothing of SyncTeX's file to the disk: a run with no records has no member there before the cross-reference table, and plain text only as its buffer fills, and a body can ask.  The run that
    writes a chain, and one that resumes from a record after an edit, against a cold run, under -synctex=1 and -synctex=-1.  Records used to settle the file, all their members on the disk, and a resume wrote the chain's
    bytes there at once."""
    bad = []
    for flag in ('-synctex=1', '-synctex=-1'):
        w = World(engine, 'synctexbusy', main='busy.tex', extra_args=(flag,))
        with open(os.path.join(w.src, 'busy.tex'), 'w') as f:
            f.write(SYNCTEX_BUSY)
        w.settle()
        w.reset_out()
        ref_run = w.cold()
        ref = state(w.out)
        t = w.write_chain(RSTEX_PAGE_CK='1')
        d = judged(ref, ref_run, state(w.out), t)
        if b'rstex: checkpoint: restored' not in t.stderr or len(record_lines(t)) < 8:
            d.append(f'{len(record_lines(t))} records written')
        if d:
            bad.append((flag, 'writing the chain', d))
        with open(os.path.join(w.src, 'busy.tex')) as f:
            text = f.read()
        with open(os.path.join(w.src, 'busy.tex'), 'w') as f:
            f.write(text.replace('Page 9 has', 'Page 9 now has'))
        w.reset_out()
        ref_run = w.cold()
        ref = state(w.out)
        w.reset_out()
        p = w.run(RSTEX_CHECKPOINT_TRACE='1', RSTEX_PAGE_CK='1')
        d = judged(ref, ref_run, state(w.out), p)
        if b'rstex: checkpoint: restored' not in p.stderr or used_record(p) < 4:
            d.append(f'resumed from record {used_record(p)}')
        if d:
            bad.append((flag, 'resumed', d))
        w.remove()
    print(f'SyncTeX read in the body: {len(bad)} wrong {bad[:4]}')
    return not bad


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
    """Answering the log of a record that then does not hold makes kpathsea read its font map for the first time, a file the .fls names where that happens.  A run from an earlier record has to name it there all the same, and that is no
    reason for it to go cold: under an editor's flags that is what a second pass after a changed .aux and an edit before the first record would pay, a cold run each and no chain left."""
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
    used = used_record(p)
    w.reset_out()
    p = w.run(RSTEX_CHECKPOINT_TRACE='1', RSTEX_PAGE_CK='1')
    d += judged(ref, ref_run, state(w.out), p)
    # The first record fails, and nothing is left to resume from: a run from the preamble.
    with open(os.path.join(w.src, 'late.tex')) as f:
        text = f.read()
    with open(os.path.join(w.src, 'late.tex'), 'w') as f:
        f.write(text.replace('First.', 'First and more.'))
    w.reset_out()
    ref_run = w.cold()
    ref = state(w.out)
    w.reset_out()
    p = w.run(RSTEX_CHECKPOINT_TRACE='1', RSTEX_PAGE_READONLY='1')
    d += judged(ref, ref_run, state(w.out), p)
    seen = seen or any('kpathsea has read' in s for s in said(p.stderr))
    print(f'what the recorder would miss: {"went cold" if seen else "no run went cold"}, the first resumed from record {used}, {len(d)} differ {d[:3]}')
    w.remove()
    return not d and not seen and used > 0


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
    """Where there is to be no chain: a run that reaches no record makes nothing, and with a switch off a run gives what it gave."""
    ok = True
    for what, main, extra_args, env in (('one page', 'hello.tex', (), {'RSTEX_PAGE_NO_DOCUMENT_END': '1'}), ('the switch', 'adv.tex', (), {'RSTEX_NO_PAGES': '1'})):
        w = World(engine, 'none', main=main, extra_args=extra_args)
        with open(os.path.join(w.src, 'hello.tex'), 'w') as f:
            f.write('\\documentclass{article}\\begin{document}Hello.\\end{document}\n')
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


def damage(args, engine, extra_args=()):
    """A flipped byte, or a file cut short, anywhere in a chain: the run sees it, or it did not matter, and the bytes are the cold run's either way.  And a record used against the log, to show that a comparison can fail at all."""
    w = World(engine, 'damage', extra_args=extra_args)
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
    print(f'damage {" ".join(extra_args)}: {trials} trials over {len(names)} files of the chain, {seen} seen as damage, {len(bad)} differ')
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
    return not bad and control and (bool(extra_args) or strange_chains(engine))


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
    env = proflib.out_env(argv, cwd, proflib.corpus().entry_env(doc.entry))
    env.update(RSTEX_CHECKPOINT_POLICY='always', XDG_CACHE_HOME=root + '/c')
    return doc, argv, cwd, env, out, src, root, pre


def run_plain(argv, cwd, env):
    p = subprocess.run(argv, cwd=cwd, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    return p


def kill(args, engine, extra_args=()):
    """SIGKILL in the middle of a run, as an editor that cancels would do it, some of them right after an edit, then an ordinary run.  It must still equal a cold one."""
    if args.doc:
        doc, argv, cwd, env, out, src, root, pre = corpus_world(engine, args.doc, 'kill')
        argv = [argv[0], *extra_args, *argv[1:]]
    else:
        w = World(engine, 'kill', extra_args=extra_args)
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
    print(f'kill {args.doc or "adv.tex"} {" ".join(extra_args)}: {args.trials} trials, {edits} edits, {len(bad)} differ; records used {used}')
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


def counted(argv, cwd, env, stderr_path=None):
    r = proflib.run_counted(argv, cwd, env, stderr_path=stderr_path)
    c = r['counters']
    return {'instr': c['instructions']['value'] / 1e6, 'cycles': c['cycles']['value'] / 1e6, 'faults': c['page-faults']['value'], 'cpu': r['cpu_ms'], 'wall': r['wall_ms']}


def session(args, engine):
    """A sitting at the editor: edits placed uniformly over the records, each kept, each run to the fixed point as latexmk would run it, with page records against the preamble's checkpoint alone.
    The two must agree at every step, and the numbers say what it was worth."""
    doc, argv, cwd, env, out, src, root, base0 = corpus_world(engine, args.doc, 'session')
    argv = [argv[0], *(args.flags or '').split(), *argv[1:]]
    caches = {'B': root + '/cB', 'A': root + '/cA'}
    for c in caches.values():
        os.makedirs(c)
    extra = {'B': {'RSTEX_NO_PAGES': '1'}, 'A': {'RSTEX_PAGE_CK': str(args.every)} if args.every else {}}
    low, high = (float(x) for x in (args.where or '0-1').split('-'))
    went_cold = [0]

    def go(which, more=None):
        trace = root + '/said'
        r = counted(argv, cwd, {**env, 'XDG_CACHE_HOME': caches[which], 'RSTEX_CHECKPOINT_TRACE': '1', **extra[which], **(more or {})}, stderr_path=trace)
        with open(trace, 'rb') as f:
            lines = said(f.read())
        m = next((m for line in lines for m in [re.match(r'pages: (\d+) records, using (\d+)', line)] if m), None)
        r['used'] = f'{m.group(2)}/{m.group(1)}' if m else '-'
        if any('kpathsea has read' in line for line in lines):
            r['used'] += ' COLD'
            went_cold[0] += 1
        return r

    def side(st):
        return {k: v for k, v in st.items() if k.endswith(SIDE)}

    # Both start from a preamble checkpoint with its seal, and the tables of kpathsea in the cache.  The chain's has said how long the document is, as a user's has.
    for which in caches:
        put(out, base0)
        run_plain(argv, cwd, {**env, 'XDG_CACHE_HOME': caches[which], **extra['B' if which == 'B' else 'A']})
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
        frac = low + (high - low) * rnd.random()
        k0 = min(int(frac * len(where)), len(where) - 1)
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
        print(f'  {step:>2} {kind:<7} at {frac:4.2f} page {page:>4} records {" ".join(x["used"] for x in pa)} passes {len(pb)}/{len(pa)}  first pass Minstr {pb[0]["instr"]:>8.0f} -> {pa[0]["instr"]:>8.0f} ({pa[0]["instr"] / pb[0]["instr"]:.2f})'
              f'  to the fixed point {sum(x["instr"] for x in pb):>8.0f} -> {sum(x["instr"] for x in pa):>8.0f} ({sum(x["instr"] for x in pa) / sum(x["instr"] for x in pb):.2f})'
              f'  cpu first {pb[0]["cpu"]:>6.0f} -> {pa[0]["cpu"]:>6.0f}  all {sum(x["cpu"] for x in pb):>6.0f} -> {sum(x["cpu"] for x in pa):>6.0f}  {"same" if not d else "DIFFERS " + str(d)}', flush=True)
    A, B = totals['A'], totals['B']
    if B[0]:
        print(f'totals: first pass instr {A[0] / B[0]:.3f}, to the fixed point {A[1] / B[1]:.3f}; cpu first pass {A[2] / B[2]:.3f}, to the fixed point {A[3] / B[3]:.3f}; {bad} of {args.steps} steps differ; {went_cold[0]} passes went cold')
    sizes = sum(len(v) for v in state(caches['A'] + '/rstex/checkpoints').values())
    print(f'the cache with a chain: {sizes / 1e6:.1f} MB')
    shutil.rmtree(root, ignore_errors=True)
    return bad == 0 and went_cold[0] == 0


def verify_one(engine, doc_id, every, flags=''):
    """The state itself: a resumed run's sums at each later record against what the writer's were."""
    doc, argv, cwd, env, out, src, root, pre = corpus_world(engine, doc_id, 'verify')
    argv = [argv[0], *flags.split(), *argv[1:]]
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
        for doc_id, n, checked, differ, b in ex.map(lambda d: verify_one(engine, d, args.every, args.flags or ''), docs):
            print(f'  {doc_id}: {n} records, {checked} states checked, {differ} differ{" " + str(b) if b else ""}', flush=True)
            bad += differ + len(b)
            total += checked
    print(f'verify: {len(docs)} documents, {total} states checked, {bad} differ')
    return bad == 0 and total > 0


def reclaim(args, engine):
    """On a build with the feature tight_heap, whose heap this document runs out of a dozen times.  A record lies between one reclaiming and the next, and a run resumed there has to reclaim where the writer did: the state is held
    against the writer's at each later record, not only the output.  On any other build this reclaims nothing and proves nothing."""
    return states_held(engine, 'reclaim.tex')


def states_held(engine, main, status=0):
    """A chain with a record on every page, and runs from some of them, whose state is held against the writer's at each later record."""
    w = World(engine, 'held', main=main)
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
    print(f'{main}: exit {ref_run.returncode}, {n} records, {checked} states checked, {differ} differ, {len(bad)} others {bad[:3]}')
    w.remove()
    return status in (None, ref_run.returncode) and not bad and not differ and (checked > 0 or n < 2)


def restart(args, engine):
    """An error in a cell is one that only real PGF can say rightly, so the job runs again from its start with the path off, in the same process, and has to find the world as the job found it: DESIGN section 10.
    A chain can have put the files it had open back as they were at a record, and the second run reads those.  So each of these is held against a run with the path off, from the same state of the output directory:
    the first pass after a chain, going on from a record, with the last PDF in place or not, under the recorder and with SyncTeX, and from a preamble checkpoint alone."""
    bad = []
    said_restart = 0

    def one(what, extra_args=(), chain=True, **env):
        nonlocal said_restart
        w = World(engine, 'restart', main='restart.tex', extra_args=extra_args)
        env = dict(env)
        # The side files of the document as it stands, and its state after a chain or a checkpoint.
        w.settle()
        # A run has its streams in the last PDF, with --keep-pdf, and those of the side files are the last run's.
        w.reset_out()
        w.run(RSTEX_NO_PAGES='1')
        w.reset_out()
        if chain:
            t = w.run(RSTEX_CHECKPOINT_TRACE='1', RSTEX_PAGE_INPUTS='1', RSTEX_PAGE_CK='2', **env)
            if not record_lines(t):
                bad.append((what, 'no chain'))
        else:
            w.run(RSTEX_NO_PAGES='1', **env)
        with open(os.path.join(w.src, 'restart.tex')) as f:
            text = f.read()
        with open(os.path.join(w.src, 'restart.tex'), 'w') as f:
            f.write(text.replace('%%EDIT%%', RESTART_EDIT))
        before = state(w.out)
        # The reference: this document with the path off, from these files.
        argv = list(w.argv)
        w.argv[1:1] = ['--no-commute']
        ref_run = w.cold(**env)
        ref = state(w.out)
        w.argv = argv
        put(w.out, before)
        p = w.run(RSTEX_CHECKPOINT_TRACE='1', RSTEX_OUTCACHE_TRACE='1', **({} if chain else {'RSTEX_NO_PAGES': '1'}), **env)
        d = judged(ref, ref_run, state(w.out), p)
        lines = said(p.stderr)
        if chain and used_record(p) == 0:
            d.append('it did not resume from a record')
        if not chain and 'restored' not in lines:
            d.append('it did not restore')
        if not any(line.startswith('the job runs again') for line in lines):
            d.append('it did not run again')
        said_restart += 1
        print(f'  {what}: {"differs " + str(d)[:300] if d else "the same as the path off"}', flush=True)
        if d:
            bad.append((what, d))
        w.remove()

    one('from a record')
    one('from a record, under the recorder', extra_args=('-recorder',))
    one('from a checkpoint', chain=False)
    one('from a checkpoint, under the recorder', extra_args=('-recorder',), chain=False)
    one('from a checkpoint, with SyncTeX', extra_args=('-synctex=1',), chain=False)
    one('from a record, with SyncTeX and the recorder', extra_args=('-synctex=1', '-recorder'))
    # The next pass is another process, with the path on again, and it goes the same way.
    one('from a checkpoint, under --until-stable', extra_args=('--until-stable',), chain=False)
    print(f'restart: {said_restart} ways, {len(bad)} differ')
    return not bad


def gate(args, engine):
    results = []
    for name, f in (('records', the_records), ('world', world), ('damage', damage), ('damage with SyncTeX', lambda a, e: damage(a, e, ('-synctex=1',))), ('kill', kill), ('kill with SyncTeX', lambda a, e: kill(a, e, ('-synctex=1', '-recorder'))),
                    ('together', together), ('clock', clock)):
        ok = f(args, engine)
        print(f'{name}: {"pass" if ok else "FAIL"}', flush=True)
        results.append((name, ok))
    ok = restart(args, engine)
    print(f'restart: {"pass" if ok else "FAIL"}', flush=True)
    results.append(('restart', ok))
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
    ap.add_argument('what', choices=['gate', 'records', 'world', 'damage', 'kill', 'together', 'clock', 'session', 'verify', 'reclaim', 'restart'])
    ap.add_argument('--engine', required=True)
    ap.add_argument('-j', '--jobs', type=int, default=4)
    ap.add_argument('--doc')
    ap.add_argument('--docs')
    ap.add_argument('--steps', type=int, default=14)
    ap.add_argument('--every', type=int, default=0, help='a record every so many pages; by default what the program does by itself')
    ap.add_argument('--seed', type=int, default=5)
    ap.add_argument('--mix', default='letter,word,word,para')
    ap.add_argument('--flags', help='given to the engine in each run of a session, as an editor would')
    ap.add_argument('--where', help='for a session: where the edits go, as a range of the records, such as 0.9-1')
    ap.add_argument('--trials', type=int, default=20)
    ap.add_argument('--flips', type=int, default=4)
    ap.add_argument('--wait', type=int, default=61)
    ap.add_argument('--keep-pdf', action='store_true', help='a run finds the PDF the last one left')
    args = ap.parse_args()
    global KEEP_PDF
    KEEP_PDF = args.keep_pdf
    os.makedirs(ROOT, exist_ok=True)
    engine = proflib.Engine('new', args.engine)
    f = {'gate': gate, 'records': the_records, 'world': world, 'damage': damage, 'kill': kill, 'together': together, 'clock': clock, 'session': session, 'verify': verify, 'reclaim': reclaim, 'restart': restart}[args.what]
    return 0 if f(args, engine) else 1


if __name__ == '__main__':
    sys.exit(main())
