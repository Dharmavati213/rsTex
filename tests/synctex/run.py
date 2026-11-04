#!/usr/bin/env -S python3 -I
r"""run.py: SyncTeX against the oracle, one requirement at a time.

    run.py --engine PATH [--filter REGEX] [-j N] [--work DIR] [--verbose]

PATH is what tests/gate.sh leaves as target/gate/ID/rstex, which finds its own formats by -fmt=pdflatex and -fmt=pdftex.

Each case of CASES below runs in a new directory, the same absolute path for both engines, one after the other, with the same stale files in place if it asks for them.  Compared:

  - the text of the file, whichever name it has, gunzipped if it is gzip: this is the requirement, and the bytes of the container are not, since DESIGN section 9 makes it members;
  - which files the run leaves, by name, and whether the file is gzip;
  - the terminal, the exit status, and the log, masked as tests/integration/run.py masks them;
  - on the candidate, the PDF against a run with SyncTeX off: a run that writes SyncTeX has the same PDF, byte for byte, and the same log, if the case changes nothing but that.

Then what the oracle cannot say: a case with `workers` is run with a worker from the first member and with none, and the file must be the same bytes; and each case is run again from a checkpoint it wrote, with
--no-snapshot's run as the judge, for those that have a preamble.  Everything is written under target/, and every document here is written new.

MUTANTS are the engine with one thing wrong in what it leaves: this tool must say so of each, or an agreement above proves nothing.
"""
import argparse
import concurrent.futures
import gzip
import importlib.util
import json
import os
import re
import shutil
import stat
import subprocess
import sys
import zlib

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
ORACLE = '/usr/bin/pdftex'
TIMEOUT = 300


def load_integration():
    spec = importlib.util.spec_from_file_location('rstex_integration_run', os.path.join(REPO, 'tests', 'integration', 'run.py'))
    mod = importlib.util.module_from_spec(spec)
    sys.modules['rstex_integration_run'] = mod
    spec.loader.exec_module(mod)
    return mod


integration = load_integration()

# ---------------------------------------------------------------------------------------------- documents, all written for this
LATEX = r'''\documentclass{article}
\begin{document}
A first line of plain words, and then $a+b=c$ in mathematics.

\input{part}

\begin{tabular}{ll}a&b\\c&d\end{tabular}
\end{document}
'''
PART = r'''Words that come from a second file, so that a tag other than the first one
appears in the nodes.  \hbox{a box} and a rule \vrule width 1pt height 2pt\relax here.
\vskip 3pt\hrule\vskip 3pt
'''
PLAIN = r'''\input part
Plain words here, with a kern\kern 3pt and a rule \vrule width 2pt\relax{} and $x^2$.
\par\vfil\eject
A second page.\par
\bye
'''
SHIPS = r'''\catcode`\{=1 \catcode`\}=2 \catcode`\#=6 \catcode`\^=7
\font\tenrm=cmr10 \tenrm \nopagenumbers
\pdfoutput=1 \pdfpagewidth=200pt \pdfpageheight=200pt
\shipout\vbox{\hrule height 1pt\hbox to 100pt{a\hfil b\kern 3pt c}\vskip 2pt\hbox{}\vbox{}}
\bye
'''
HEAD = r'''\catcode`\{=1 \catcode`\}=2 \catcode`\#=6 \catcode`\^=7 \catcode`\$=3
\font\tenrm=cmr10 \tenrm \textfont0=\tenrm \scriptfont0=\tenrm \scriptscriptfont0=\tenrm
\pdfoutput=1 \pdfpagewidth=200pt \pdfpageheight=200pt \hsize=150pt \parindent=0pt \baselineskip=12pt
'''
FORMS = HEAD + r'''\setbox0=\hbox{in a form \kern 2pt x}
\immediate\pdfxform 0
\edef\first{\the\pdflastxform}
\setbox0=\vbox{\hbox{another}\hrule}
\pdfxform 0
\edef\second{\the\pdflastxform}
\shipout\vbox{\hbox{before \pdfrefxform\first\ after \pdfrefxform\second}\pdfrefxform\first}
\shipout\vbox{\pdfrefxform\second}
\end
'''
LISTS = HEAD + r'''\TeXXeTstate=1
\def\page{\shipout\vbox{%
  \hbox to 100pt{left\hfil\kern 5pt right}%
  \hbox to 100pt{\leaders\hrule\hfil}%
  \hbox to 100pt{\leaders\hbox{.\kern 2pt}\hfil}%
  \hbox{\vrule width 3pt height 3pt depth 1pt\vrule}%
  \hrule height 1pt depth 1pt
  \leaders\vrule\vskip 4pt
  \hbox{\box1 \copy2 \hskip 2pt plus 1fil}%
  \vbox to 20pt{\vss\hbox{\beginR reversed \kern 2pt\endR}\vss}%
  \hbox{$x$\kern 1pt$$}\hbox{}\vbox{}\moveright 3pt\hbox{moved}\hbox{\raise 2pt\hbox{}\lower 1pt\vbox{}}%
  \hbox{\kern 1pt\kern 2pt\hskip 3pt\kern 4pt}%
}}
\setbox1=\hbox{}\setbox2=\vbox{}
\page
\setbox1=\hbox to 5pt{}\setbox2=\vbox to 5pt{}
\page
\end
'''
LINES = HEAD + r'''\hbox{one}
\hbox{two}\kern 3pt

\hbox{three}
\vfil\penalty-10000
Words on several
lines of the source, so that each line
has its own record in what is shipped out, as a paragraph
that goes on and on and on for more than one line of the page.
\par\vfil\penalty-10000
\end
'''
MANY = HEAD + r'''\def\w{words words words words words words words words words words words words words words words }
\newcount\k \k=0
\loop\advance\k by 1
\shipout\vbox{\hsize=150pt \w\w\w\w\w\w\w\w\w\w\w\w\w\w\w\w\w\w\w\w\w\w\w\w\w\w\w\w\w\w\w\w\w\w\w\w\w\w\w\w\w\w\w\w\w\w\w\w\w\w\par \hbox{page \the\k}}
\ifnum\k<40 \repeat
\end
'''.replace(r'\newcount\k \k=0', r'\countdef\k=10 \k=0').replace(r'\loop', r'\def\again{').replace(r'\ifnum\k<40 \repeat', r'\ifnum\k<40 \expandafter\again\fi}\again')


def plain(body):
    return HEAD + body + '\n\\end\n'


def c(name, files, args=None, main=None, fmt='pdftex', stale=(), ls=(), workers=False, on_off=True, warm=True, stdin=None):
    """`files` is {name: text}; `main` the file named on the command line, or a first line; `args` what comes before it; `stale` files that lie there already."""
    return {'name': name, 'files': files, 'args': args if args is not None else ['-synctex=1'], 'main': main if main is not None else next(iter(files)), 'fmt': fmt, 'stale': list(stale),
            'workers': workers, 'on_off': on_off, 'warm': warm, 'stdin': stdin}


GZ, TXT = 'doc.synctex.gz', 'doc.synctex'
ALL_STALE = (GZ, TXT, 'doc.synctex(busy)')
DOC = {'doc.tex': PLAIN, 'part.tex': PART}
CASES = [
    # The signs and bits of the option, as texmfmp.c and synctex.c read them.
    c('option/one', DOC),
    c('option/two', DOC, ['-synctex=2']),
    c('option/three', DOC, ['-synctex=3']),
    c('option/forms_bit', DOC, ['-synctex=4']),
    c('option/compress_bit', DOC, ['-synctex=8']),
    c('option/all', DOC, ['-synctex=15']),
    c('option/high', DOC, ['-synctex=64']),
    c('option/minus_one', DOC, ['-synctex=-1']),
    c('option/minus_two', DOC, ['-synctex=-2']),
    c('option/minus_all', DOC, ['-synctex=-15']),
    c('option/hex', DOC, ['-synctex=0x9']),
    c('option/twice', DOC, ['-synctex=1', '-synctex=-1']),
    c('option/zero', DOC, ['-synctex=0'], on_off=False),
    c('option/none', DOC, [], on_off=False),
    c('option/letters', DOC, ['-synctex=x'], on_off=False),
    # A stale file: every run that ends takes it away, with or without SyncTeX, and what it takes depends on the sign.
    c('stale/off', DOC, [], stale=ALL_STALE, on_off=False),
    c('stale/zero', DOC, ['-synctex=0'], stale=ALL_STALE, on_off=False),
    c('stale/one', DOC, stale=ALL_STALE),
    c('stale/minus_one', DOC, ['-synctex=-1'], stale=ALL_STALE),
    c('stale/two', DOC, ['-synctex=2'], stale=ALL_STALE),
    c('stale/no_pages', {'doc.tex': '\\input part\n\\end\n', 'part.tex': '\n'}, stale=ALL_STALE),
    c('stale/fatal', {'doc.tex': '\\input part\n\\shipout\\hbox{}\n\\undefinedname\n\\bye\n', 'part.tex': '\n'}, ['-synctex=1', '-halt-on-error'], stale=ALL_STALE),
    c('stale/fatal_off', {'doc.tex': '\\undefinedname\n\\bye\n'}, ['-halt-on-error'], stale=ALL_STALE, on_off=False),
    # No pages, an interaction mode, the names.
    c('end/no_pages', {'doc.tex': '\\input part\n\\end\n', 'part.tex': '\n'}),
    c('end/no_page_no_file', {'doc.tex': '\\end\n'}),
    c('end/batch', DOC, ['-synctex=1', '-interaction=batchmode']),
    c('end/scroll', DOC, ['-synctex=1', '-interaction=scrollmode']),
    c('end/batch_in_the_file', {'doc.tex': '\\batchmode\n' + PLAIN, 'part.tex': PART}),
    c('end/dump', {'doc.tex': '\\catcode`\\{=1 \\catcode`\\}=2 \\input part\n\\dump\n', 'part.tex': '\n'}, ['-ini', '-synctex=1'], on_off=False),
    c('end/busy_is_a_directory', DOC, stale=('doc.synctex(busy)/',)),
    c('end/real_is_a_directory', DOC, stale=('doc.synctex.gz/',), on_off=False),
    c('end/real_is_a_full_directory', DOC, stale=('doc.synctex.gz/inner',), on_off=False),
    c('end/draftmode', DOC, ['-synctex=1', '-draftmode'], on_off=False),
    c('end/error_stop', {'doc.tex': '\\input part\n\\shipout\\hbox{}\n\\undefinedname\n\\bye\n', 'part.tex': '\n'}, ['-synctex=1'], stdin=b'', on_off=False),
    c('name/jobname', DOC, ['-synctex=1', '-jobname=other']),
    c('name/output_directory', dict(DOC), ['-synctex=1', '-output-directory=out'], stale=('out/doc.synctex.gz',)),
    c('name/output_directory_off', dict(DOC), ['-output-directory=out'], stale=('out/doc.synctex.gz', 'doc.synctex.gz'), on_off=False),
    c('name/space', {'my doc.tex': PLAIN, 'part.tex': PART}, main='my doc.tex'),
    c('name/space_minus', {'my doc.tex': PLAIN, 'part.tex': PART}, ['-synctex=-1'], main='my doc.tex'),
    c('name/dots', {'a.b.c.tex': PLAIN, 'part.tex': PART}, main='a.b.c'),
    c('name/first_line_no_file', {'part.tex': PART}, main=r'\input part \shipout\hbox{}\end'),
    c('name/first_line_ship', {}, main=r'\shipout\hbox{}\end'),
    c('name/first_line_input_subdir', {'sub/part.tex': PART + '\\bye\n'}, main=r'\input sub/part'),
    c('name/dot_slash', {'doc.tex': PLAIN, 'part.tex': PART}, main='./doc.tex'),
    # What the primitive does: the option gives it a value, and it can be read and changed.
    c('primitive/show', {'doc.tex': '\\message{[\\the\\synctex]}\\input part\n\\showthe\\synctex\n\\shipout\\hbox{}\\bye\n', 'part.tex': '\n'}, ['-synctex=1', '-interaction=nonstopmode'], on_off=False),
    c('primitive/show_two', {'doc.tex': '\\message{[\\the\\synctex]}\\shipout\\hbox{}\\bye\n'}, ['-synctex=2'], on_off=False),
    c('primitive/show_minus', {'doc.tex': '\\message{[\\the\\synctex]}\\shipout\\hbox{}\\bye\n'}, ['-synctex=-4'], on_off=False),
    c('primitive/show_none', {'doc.tex': '\\message{[\\the\\synctex]}\\shipout\\hbox{}\\bye\n'}, [], on_off=False),
    c('primitive/in_file', {'doc.tex': '\\synctex=1\n' + PLAIN, 'part.tex': PART}, [], on_off=False),
    c('primitive/late_in_file', {'doc.tex': '\\input part\n\\synctex=1\n\\input part\n\\shipout\\hbox{a}\n\\bye\n', 'part.tex': '\n'}, [], on_off=False),
    c('primitive/just_before_ship', {'doc.tex': '\\input part\n\\shipout\\hbox{}\n\\synctex=1\n\\shipout\\hbox{}\n\\bye\n', 'part.tex': '\n'}, [], on_off=False),
    c('primitive/in_file_minus', {'doc.tex': '\\synctex=-1\n' + PLAIN, 'part.tex': PART}, [], on_off=False),
    c('primitive/zero_refused', {'doc.tex': '\\synctex=1\n' + PLAIN, 'part.tex': PART}, ['-synctex=0'], on_off=False),
    c('primitive/off_for_a_while', {'doc.tex': '\\input part\n\\shipout\\hbox{a}\n\\synctex=0\n\\input part\n\\shipout\\hbox{b}\n\\synctex=1\n\\shipout\\hbox{c}\n\\bye\n', 'part.tex': '\n'}, on_off=False),
    c('primitive/off_before_first', {'doc.tex': '\\synctex=0\n\\input part\n\\shipout\\hbox{a}\n\\synctex=1\n\\shipout\\hbox{b}\n\\bye\n', 'part.tex': '\n'}, on_off=False),
    c('primitive/in_a_group', {'doc.tex': '\\begingroup\\synctex=0 \\shipout\\hbox{a}\\endgroup\n\\shipout\\hbox{b}\\bye\n'}, on_off=False),
    c('primitive/first_file_after_on', {'doc.tex': '\\synctex=1 \\input part\n\\shipout\\hbox{a}\\bye\n', 'part.tex': '\\synctex=0\n\\input part2\n', 'part2.tex': '\\synctex=1\n'}, [], on_off=False),
    # DVI: the offsets, and the magnification.
    c('dvi/plain', DOC, ['-synctex=1', '-output-format=dvi']),
    c('dvi/minus', DOC, ['-synctex=-1', '-output-format=dvi']),
    c('dvi/in_file', {'doc.tex': '\\pdfoutput=0\n' + PLAIN, 'part.tex': PART}),
    c('dvi/mag', {'doc.tex': '\\mag=2000\n' + PLAIN, 'part.tex': PART}, ['-synctex=1', '-output-format=dvi']),
    c('mag/pdf', {'doc.tex': '\\mag=2000\n' + PLAIN, 'part.tex': PART}),
    c('mag/after_first', {'doc.tex': '\\input part\n\\shipout\\hbox{a}\n\\mag=2000\n\\shipout\\hbox{b}\n\\bye\n', 'part.tex': '\n'}, ['-synctex=1', '-interaction=nonstopmode'], on_off=False),
    # What ship_out sends.
    c('ship/boxes', {'doc.tex': SHIPS}),
    c('ship/lists', {'doc.tex': LISTS}),
    c('ship/lists_compress', {'doc.tex': LISTS}, ['-synctex=8']),
    c('ship/lists_dvi', {'doc.tex': LISTS}, ['-synctex=1', '-output-format=dvi']),
    c('ship/lines', {'doc.tex': LINES}),
    c('ship/forms', {'doc.tex': FORMS}),
    c('ship/forms_with_bit', {'doc.tex': FORMS}, ['-synctex=4']),
    c('ship/forms_compress', {'doc.tex': FORMS}, ['-synctex=12']),
    c('ship/huge_page', {'doc.tex': HEAD + '\\input part\n\\shipout\\vbox to 20000pt{}\n\\shipout\\hbox{a}\n\\end\n', 'part.tex': '\n'}, ['-synctex=1', '-interaction=nonstopmode'], on_off=False),
    c('ship/scantokens', {'doc.tex': HEAD + '\\everyeof{\\noexpand}\\shipout\\hbox{\\scantokens{in a pseudo file}\\hbox{}}\n\\end\n'}),
    c('ship/read_terminal', {'doc.tex': HEAD + '\\read-1 to \\x \\shipout\\hbox{\\x\\kern 1pt}\n\\end\n'}, stdin=b'typed\n'),
    c('ship/nested_inputs', {'doc.tex': HEAD + '\\input a \\shipout\\hbox{\\kern 1pt}\\input b \\shipout\\hbox{\\kern 2pt}\\end\n', 'a.tex': '\\input b\n', 'b.tex': '\\hbox{}\n'}),
    c('ship/tag_is_of_the_file', {'doc.tex': HEAD + '\\def\\m{\\hbox{m}\\kern 1pt\\hskip 1pt}\\input a \\input b \\shipout\\vbox{\\box1 \\box2 \\m}\\end\n', 'a.tex': '\\setbox1=\\hbox{\\m}\n', 'b.tex': '\n\n\n\\setbox2=\\hbox{\\m}\n'}),
    c('ship/big_positions', {'doc.tex': HEAD + '\\pdfhorigin=15000pt \\pdfpagewidth=16000pt \\shipout\\hbox{\\kern -14000pt \\hbox{}\\kern 16000pt \\hbox{}}\n\\shipout\\vbox{\\moveleft 9000pt\\hbox{}\\kern -9000pt \\hbox{}}\n\\end\n'}),
    c('ship/in_preamble_form', {'doc.tex': HEAD + '\\setbox0=\\hbox{a}\\immediate\\pdfxform 0 \\input part\n\\shipout\\hbox{\\kern 1pt}\\end\n', 'part.tex': '\n'}),
    # Many members.  The file must be the same bytes with workers and without.
    c('many/pages', {'doc.tex': MANY}, workers=True),
    c('many/pages_minus', {'doc.tex': MANY}, ['-synctex=-1'], workers=True),
    c('many/pages_dvi', {'doc.tex': MANY}, ['-synctex=1', '-output-format=dvi'], workers=True),
    # LaTeX: the preamble's inputs, the .aux read twice, and a run from a checkpoint.
    c('latex/hello', {'doc.tex': LATEX, 'part.tex': PART}, fmt='pdflatex'),
    c('latex/minus', {'doc.tex': LATEX, 'part.tex': PART}, ['-synctex=-1'], fmt='pdflatex'),
    c('latex/in_preamble', {'doc.tex': '\\synctex=1\n' + LATEX, 'part.tex': PART}, [], fmt='pdflatex', on_off=False),
    c('latex/in_body', {'doc.tex': LATEX.replace('\\begin{document}', '\\begin{document}\\synctex=1 '), 'part.tex': PART}, [], fmt='pdflatex', on_off=False),
    c('latex/recorder', {'doc.tex': LATEX, 'part.tex': PART}, ['-synctex=1', '-recorder'], fmt='pdflatex'),
    c('latex/package_says', {'doc.tex': '\\documentclass{article}\\usepackage{hyperref}\\begin{document}a\\newpage b\\end{document}\n'}, fmt='pdflatex'),
    c('latex/tag_counter_in_a_checkpoint', {'doc.tex': '\\documentclass{article}\\makeatletter\\@input{part}\\makeatother\\begin{document}\\input{part}a\\end{document}\n', 'part.tex': '\\relax\n'}, fmt='pdflatex'),
]

FMT_ARGS = {'pdftex': ['-fmt=pdftex', '-progname=pdftex'], 'pdflatex': ['-fmt=pdflatex', '-progname=pdflatex']}


def fresh(work, case):
    shutil.rmtree(work, ignore_errors=True)
    os.makedirs(work)
    for name, text in case['files'].items():
        path = os.path.join(work, name)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, 'w') as f:
            f.write(text)
    if any(a.startswith('-output-directory') for a in case['args']):
        os.makedirs(os.path.join(work, 'out'), exist_ok=True)
    for name in case['stale']:
        path = os.path.join(work, name)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        # A name with a slash is a directory in its way, and that is how a file comes not to open or rename.
        if not name.endswith('/'):
            with open(path, 'w') as f:
                f.write('stale\n')


def run(engine, case, work, env, args=None, extra=()):
    argv = [engine] + list(extra) + FMT_ARGS[case['fmt']] + (case['args'] if args is None else args) + ([case['main']] if case['main'] else [])
    stdin = subprocess.DEVNULL if case['stdin'] is None else None
    p = subprocess.run(argv, cwd=work, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, stdin=stdin, input=case['stdin'], timeout=TIMEOUT)
    return p


def snapshot(work):
    out = {}
    for root, _, names in os.walk(work):
        for n in names:
            p = os.path.join(root, n)
            with open(p, 'rb') as f:
                out[os.path.relpath(p, work)] = f.read()
    return out


def sync_files(files):
    """The name, the container and the text of every SyncTeX file, a stale one as it is."""
    out = {}
    for name, data in files.items():
        if '.synctex' in name:
            if data[:2] == b'\x1f\x8b':
                try:
                    out[name] = ('gzip', gzip.decompress(data))
                except (OSError, EOFError, zlib.error):
                    out[name] = ('broken gzip', b'')
            else:
                out[name] = ('text', data)
    return out


def first_diff(a, b):
    for i, (x, y) in enumerate(zip(a.split(b'\n'), b.split(b'\n'))):
        if x != y:
            return f'line {i + 1}: {x[:90]!r} and {y[:90]!r}'
    return f'lengths {len(a)} and {len(b)}'


def masked(data):
    return integration.mask_lines(data, {}, True)


def outputs_same(a, b):
    """Terminal or log: masked, and with the length of a PDF left out, which goes with its banner."""
    drop = re.compile(r'\(\d+ pages?, [^)]*\)')
    x = [drop.sub('', s) for s in masked(a)]
    y = [drop.sub('', s) for s in masked(b)]
    return x == y


def clean_env(env, **more):
    e = dict(env)
    e.update(more)
    return e


def one_case(case, engine, root, env):
    name = case['name']
    work = os.path.join(root, 'w', name.replace('/', '_'))
    problems = []
    fresh(work, case)
    o = run(ORACLE, case, work, env)
    of = snapshot(work)
    fresh(work, case)
    # A run that judges the first pass of the literal engine: no cache to find.
    cenv = clean_env(env, RSTEX_NO_SNAPSHOT='1', RSTEX_NO_COMMUTE='1')
    r = run(engine, case, work, cenv)
    rf = snapshot(work)
    if o.returncode != r.returncode:
        problems.append(f'exit status {o.returncode} and {r.returncode}')
    if not outputs_same(o.stdout, r.stdout):
        problems.append('terminal: ' + first_diff(b'\n'.join(s.encode('latin-1') for s in masked(o.stdout)), b'\n'.join(s.encode('latin-1') for s in masked(r.stdout))))
    if integration.STDERR_DROP.sub(b'', o.stderr) != integration.STDERR_DROP.sub(b'', r.stderr):
        problems.append('stderr: ' + first_diff(o.stderr, r.stderr))
    # Names: a format the case builds or a PDF is there on both, and nothing else may differ.
    if sorted(of) != sorted(rf):
        problems.append(f'files: only the oracle has {sorted(set(of) - set(rf))}, only rstex has {sorted(set(rf) - set(of))}')
    os_ = sync_files(of)
    rs_ = sync_files(rf)
    for k in sorted(set(os_) & set(rs_)):
        if os_[k][0] != rs_[k][0]:
            problems.append(f'{k}: {os_[k][0]} and {rs_[k][0]}')
        elif os_[k][1] != rs_[k][1]:
            problems.append(f'{k}: ' + first_diff(os_[k][1], rs_[k][1]))
    for k in sorted(n for n in set(of) & set(rf) if n.endswith('.log')):
        if not outputs_same(of[k], rf[k]):
            problems.append(f'{k}: ' + first_diff(b'\n'.join(s.encode('latin-1') for s in masked(of[k])), b'\n'.join(s.encode('latin-1') for s in masked(rf[k]))))
    # The recorder does not hear of this file, on the oracle.
    for k in sorted(n for n in rf if n.endswith('.fls')):
        if (b'synctex' in of.get(k, b'')) != (b'synctex' in rf[k]):
            problems.append(f'{k}: the recorder and the file')
    text_bytes = sum(len(v[1]) for v in rs_.values())
    # On the candidate, the same with SyncTeX off: PDF, DVI and log the same bytes.
    if case['on_off'] and rs_:
        fresh(work, case)
        off_args = [a for a in case['args'] if not a.startswith('-synctex')]
        run(engine, case, work, cenv, args=off_args)
        off = snapshot(work)
        for k in sorted(n for n in set(off) & set(rf) if n.endswith(('.pdf', '.dvi', '.log', '.aux'))):
            if off[k] != rf[k]:
                problems.append(f'{k} is not what a run with SyncTeX off gives: ' + (first_diff(off[k], rf[k]) if not k.endswith('.pdf') else 'bytes'))
    # Workers: from the first member, and none.
    if case['workers']:
        files = {}
        for n in ('0', '2'):
            fresh(work, case)
            run(engine, case, work, clean_env(cenv, RSTEX_DEFLATE_WORKERS=n))
            files[n] = snapshot(work)
        for k in sorted(n for n in rf if '.synctex' in n):
            if files['0'].get(k) != rf[k] or files['2'].get(k) != rf[k]:
                problems.append(f'{k}: the bytes depend on the workers')
        if not any(n.endswith('.gz') for n in rf) and not any(n.endswith('.synctex') for n in rf):
            problems.append('no file from a case about the file')
    # A checkpoint.
    restored = None
    if case['warm'] and case['fmt'] == 'pdflatex':
        cache = os.path.join(work + '-cache')
        shutil.rmtree(cache, ignore_errors=True)
        os.makedirs(cache)
        wenv = clean_env(cenv, XDG_CACHE_HOME=cache, RSTEX_CHECKPOINT_POLICY='always', RSTEX_CHECKPOINT_TRACE='1')
        wenv.pop('RSTEX_NO_SNAPSHOT')
        fresh(work, case)
        cold = run(engine, case, work, cenv)
        cf = snapshot(work)
        for label in ('write', 'restore'):
            fresh(work, case)
            w = run(engine, case, work, wenv)
            wf = snapshot(work)
            if label == 'restore':
                restored = b'rstex: checkpoint: restored' in w.stderr
            if wf != cf or w.stdout != cold.stdout or w.returncode != cold.returncode:
                bad = sorted(k for k in set(wf) | set(cf) if wf.get(k) != cf.get(k))
                problems.append(f'the run that does {label} differs from a cold one: {bad or "terminal or status"}')
        # The option is in a checkpoint's key: a run with another must not restore it.
        if restored and case['args']:
            fresh(work, case)
            w = run(engine, case, work, wenv, args=[a for a in case['args'] if not a.startswith('-synctex')] + ['-synctex=5'])
            if b'rstex: checkpoint: restored' in w.stderr:
                problems.append('a checkpoint was restored under another -synctex')
    said = f'status {o.returncode}, files {sorted(os_)}, {text_bytes} bytes of text, {sum(v[1].count(b"Input:") for v in os_.values())} inputs, {sum(v[1].count(bytes([10])) for v in os_.values())} lines, terminal says {sum(o.stdout.count(w) for w in (b"SyncTeX", b"Synchronize"))} times of it'
    return {'name': name, 'problems': problems, 'text_bytes': text_bytes, 'restored': restored, 'has_file': bool(rs_), 'said': said}


# What a mutant does after the real engine has run, in the directory it ran in: (name, the case that must then fail, code).
MUTANTS = [
    ('a digit of a record', 'option/one', """
d = gzip.decompress(open(gz, 'rb').read())
k = d.index(b'\\nk') + 2
k = d.index(b'\\n', k) - 1
d = d[:k] + (b'1' if d[k:k + 1] != b'1' else b'2') + d[k + 1:]
open(gz, 'wb').write(gzip.compress(d))
"""),
    ('an anchor one off', 'option/one', """
d = gzip.decompress(open(gz, 'rb').read())
k = d.index(b'\\n!') + 2
d = d[:k] + (b'1' if d[k:k + 1] != b'1' else b'2') + d[k + 1:]
open(gz, 'wb').write(gzip.compress(d))
"""),
    ('the last member lost', 'option/one', """
d = open(gz, 'rb').read()
open(gz, 'wb').write(d[:d.rindex(bytes([0x1f, 0x8b, 8]))])
"""),
    ('a file cut short', 'option/one', """
d = open(gz, 'rb').read()
open(gz, 'wb').write(d[:len(d) - 5])
"""),
    ('the busy file left behind', 'option/one', """
shutil.copy(gz, 'doc.synctex(busy)')
"""),
    ('a stale file kept', 'stale/one', """
open('doc.synctex', 'w').write('stale')
"""),
    ('a stale file kept by a run with no SyncTeX', 'stale/off', """
open('doc.synctex.gz', 'w').write('stale')
"""),
    ('the file gone', 'option/one', """
os.unlink(gz)
"""),
    ('plain text for gzip', 'option/one', """
d = gzip.decompress(open(gz, 'rb').read())
open(gz, 'wb').write(d)
"""),
    ('a word in the log', 'option/one', """
open('doc.log', 'a').write('SyncTeX written on doc.synctex.gz.' + chr(10))
"""),
]
MUTANT_WRAPPER = """#!/usr/bin/env -S python3 -I
import gzip, os, shutil, subprocess, sys
p = subprocess.run([@ENGINE@] + sys.argv[1:])
gz = 'doc.synctex.gz'
if sys.argv[1:] and os.path.exists(os.path.join(os.getcwd(), 'doc.log')):
@CODE@
sys.exit(p.returncode)
"""


def mutants(engine, root, env):
    """Each mutant, which the case named has to catch."""
    bad = 0
    os.makedirs(root, exist_ok=True)
    by_name = {x['name']: x for x in CASES}
    for k, (name, case, code) in enumerate(MUTANTS):
        path = os.path.join(root, f'mutant-{k}')
        body = '\n'.join('    ' + line for line in code.strip().split('\n'))
        with open(path, 'w') as f:
            f.write(MUTANT_WRAPPER.replace('@ENGINE@', repr(engine)).replace('@CODE@', body))
        os.chmod(path, os.stat(path).st_mode | stat.S_IXUSR)
        r = one_case(dict(by_name[case], workers=False, on_off=False, warm=False), path, os.path.join(root, f'mutant-{k}-work'), env)
        if not r['problems']:
            print(f'FAIL the mutant "{name}" got through {case}')
            bad += 1
    print(f'{len(MUTANTS) - bad}/{len(MUTANTS)} mutants caught')
    return bad


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--engine', required=True)
    ap.add_argument('--filter')
    ap.add_argument('--verbose', action='store_true', help='say what each case came to on the oracle, since a case that does not do what it is for agrees all the same')
    ap.add_argument('-j', type=int, default=4)
    ap.add_argument('--work', default=os.path.join(REPO, 'target', 'synctex'))
    args = ap.parse_args()
    env = dict(integration.ENVIRONMENT)
    for k in ('PATH', 'HOME'):
        if k in os.environ:
            env[k] = os.environ[k]
    cases = [x for x in CASES if not args.filter or re.search(args.filter, x['name'])]
    os.makedirs(args.work, exist_ok=True)
    engine = os.path.abspath(args.engine)
    with concurrent.futures.ThreadPoolExecutor(args.j) as ex:
        results = list(ex.map(lambda x: one_case(x, engine, args.work, env), cases))
    bad = [r for r in results if r['problems']]
    if args.verbose:
        for r in results:
            print(f'{r["name"]}: {r["said"]}')
    for r in bad:
        print(f'FAIL {r["name"]}')
        for p in r['problems']:
            print(f'     {p}')
    with_file = sum(r['has_file'] for r in results)
    restored = sum(1 for r in results if r['restored'])
    tried = sum(1 for r in results if r['restored'] is not None)
    print(f'{len(results) - len(bad)}/{len(results)} cases agree with the oracle; {with_file} of them have a file, of {sum(r["text_bytes"] for r in results)} bytes of text in all; {restored} of {tried} tried from a checkpoint were restored')
    if restored == 0 and tried:
        print('FAIL no case was restored from a checkpoint: that proves nothing')
        return 1
    caught = 0 if args.filter else mutants(engine, os.path.join(args.work, 'mutants'), env)
    print('PASS' if not bad and not caught else 'FAIL')
    return 1 if bad or caught else 0


if __name__ == '__main__':
    sys.exit(main())
