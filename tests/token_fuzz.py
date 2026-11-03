#!/usr/bin/env -S python3 -I
"""token_fuzz.py: random programs for the token machine, on the oracle and on an engine; log, terminal, exit status and PDF compared.

    python3 -I tests/token_fuzz.py ENGINE [--gen A,B] [--first SEED] [--count N] [-j N] [--run ID] [--no-reduce] [--format clean|outer] [--small]
    python3 -I tests/token_fuzz.py ENGINE --again DIR       the case kept in DIR, once more, with the first difference
    python3 -I tests/token_fuzz.py ENGINE --reduce DIR      reduces the case kept in DIR
    python3 -I tests/token_fuzz.py --list                   the generators

Made to find an input on which a fast path of get_next, capture.rs, scan.rs and their neighbours does something the literal code would not.
Each generator aims at the corners of one area, and most of what it writes is an error, which is the point: an error shows the input stack, and a run goes
on from it.  A generator takes its seeds from a stream of its own, `random.Random("GEN:SEED")`.  A document runs on `-ini -etex`, or with --format on a small format that each program has made of its own, with an \\outer macro in it or none, in the same directory for
both programs, and is judged on what tests/integration/run.py judges: the log and the terminal through its masks, the exit status, the PDF
(here byte for byte, since nothing is compressed, with the banner and the cross-reference table left out), and any other file written.

With --small each document also takes capacities of its own on the command line, near the lowest allowed: the input stack, the parameter stack, the buffer, the save stack, the nest, the
open files, the depth of expansion, and error_line and its like.  So that a stack ends at a token of its choosing in small documents, and where it ends says that no level was left out that would
have raised a high water mark.

The capacities of `main memory`, `hash size`, `pool size` and `number of strings` differ from the oracle's by design (PORTING section 15), so a document that reaches one of
these on either side is passed over and counted.  Any other capacity is the same number on both, and such a document is compared like the rest.  One
that takes both programs past --timeout, or past the limit on a file's size, is passed over too; on one only, it differs.  So is one whose first difference has on the
oracle's side a value from 2^31 up, as a number or as a dimension from 16384pt up: the port clamps those (PORTING section 15).  Each of these is counted apart.
A document that differs once and not when it is run again is counted as a flake and not kept: a loaded machine has shown one, the oracle's side as often as the engine's, each
run failing to find a font in its map.

Writes only under target/token-fuzz/ID, and keeps there, in kept/GEN-SEED, a document that differs, with both logs and what its reduction left.  Exit status: 1 if any differs.
"""
import argparse
import importlib.util
import multiprocessing
import os
import random
import re
import resource
import shutil
import signal
import subprocess
import sys
import time

sys.dont_write_bytecode = True
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ORACLE = '/usr/bin/pdftex'
spec = importlib.util.spec_from_file_location('integration_run', os.path.join(ROOT, 'tests', 'integration', 'run.py'))
integration = importlib.util.module_from_spec(spec)
spec.loader.exec_module(integration)

OUTPUT_LIMIT = 24 << 20
CAPACITY = re.compile(r'^! TeX capacity exceeded, sorry \[([^=\]]*)(?:=(\w+))?\]')
# What differs from the oracle by design: PORTING section 15, "Numbers in overflow messages".
OTHER_CAPACITY = {'main memory size', 'hash size', 'pool size', 'number of strings', 'font memory', 'pattern memory', 'pattern memory ops', 'primitive size', 'exception dictionary'}


BEYOND = re.compile(r'(?<![\d.])\d+(?:\.\d+)?')


def past_the_clamp(number):
    """The oracle holds a value from 2^31 up, as a number or in scaled points, and the port clamps it: PORTING section 15, "Values at or above 2^31".  A dimension from
    half of that is taken as one, since what shows may be a sum with that in it, or an excess over \\hsize."""
    return float(number) >= (16384.0 if '.' in number else 2147483648.0)


class Case:
    """A document: its files by name, f.tex first, what the terminal types, and how the run starts."""

    def __init__(self, files, stdin=b'', interaction='batchmode', options=()):
        self.files, self.stdin, self.interaction, self.options = files, stdin, interaction, list(options)


class Rand(random.Random):
    def chance(self, p):
        return self.random() < p

    def pick(self, *items):
        return self.choice(items)

    def some(self, atoms, a, b, sep=''):
        return sep.join(self.choice(atoms) for _ in range(self.randint(a, b)))


PREAMBLE = r'''\catcode`\{=1 \catcode`\}=2 \catcode`\#=6 \catcode`\^=7 \catcode`\&=4 \catcode`\$=3 \catcode`\_=8 \catcode`\~=13 \catcode`\@=11
\pdfoutput=1 \pdfcompresslevel=0 \pdfobjcompresslevel=0 \pdftrailerid{}
\font\f=cmr10 \f \hsize=200pt \vsize=300pt \parindent=5pt \tolerance=10000 \hbadness=10000 \vbadness=10000 \hfuzz=1000pt \vfuzz=1000pt
\maxdepth=2pt \baselineskip=12pt \errorcontextlines=20 \tracingstats=1 \maxdeadcycles=5
\def\x{}\def\y{}\def~{}\def\space{ }\let\amp=&\let\hash=#\let\lb={\let\rb=}\let\crr=\cr\let\spn=\span\let\yfi=\fi\let\sp= \protected\def\pr{}
'''
ENDING = ['\\end\n', '\\par\\end\n', '\\fi\\fi\\end\n', '}\\end\n', '\\egroup\\end\n', '\\par\\vfill\\penalty-10000 \\end\n', '\\endgroup\\end\n\\end\n', '\n']
LETTERS = 'abcdefghijklmnopqrstuvwxyz'


def label(i):
    return LETTERS[i // 676 % 26] + LETTERS[i // 26 % 26] + LETTERS[i % 26]


class Doc:
    """A document in the making.  `deliver` has a piece of source read from a line of the file, a macro's body, an argument, a register and the like."""

    def __init__(self, r, preamble=PREAMBLE):
        self.r, self.lines, self.n = r, [preamble], 0

    def add(self, text):
        self.lines.append(text)

    def name(self, prefix='\\D'):
        self.n += 1
        return prefix + label(self.n)

    def deliver(self, body, plain=0.2):
        r, d = self.r, self.name()
        hashes = body.replace('#', '##')
        k = r.random()
        if k < plain:
            return body + '\n'
        if k < 0.4:
            return '\\def%s{%s}%s\n' % (d, hashes, d)
        if k < 0.5:
            return '\\def%s#1{#1}%s{%s}\n' % (d, d, body)
        if k < 0.58:
            return '\\def%s#1#2#3{#2#1#3}%s{}{%s}{}\n' % (d, d, body)
        if k < 0.66:
            e = self.name()
            return '\\def%s#1{%s{#1}}\\def%s#1{#1}%s{%s}\n' % (d, e, e, d, body) if r.chance(0.5) else '\\def%s#1{#1}\\def%s{%s{%s}}%s\n' % (d, e, d, hashes, e)
        if k < 0.74:
            return '\\toks0={%s}\\the\\toks0 \n' % body
        if k < 0.8:
            return '\\scantokens{%s}\n' % body
        if k < 0.86:
            return '\\def%s#1.{#1}%s%s.\n' % (d, d, body)
        if k < 0.92:
            return '\\long\\def%s#1{\\expandafter\\relax#1}%s{%s}\n' % (d, d, body)
        return '\\everypar={%s}\\noindent\\leavevmode\\everypar={}\n' % body

    def text(self, stop=True):
        if stop:
            self.add(self.r.choice(ENDING))
        return ''.join(self.lines)


def single(text, r, stdin=b''):
    return Case({'f.tex': text}, stdin, r.pick('batchmode', 'batchmode', 'nonstopmode', 'scrollmode'))


# ---------------------------------------------------------------------------------------------------------------------
# macros: random parameter texts, partial matches, parameters in bodies and groups, and calls in boxes, cells, an \\edef and skipped text.
# ---------------------------------------------------------------------------------------------------------------------
M_ATOMS = ['a', 'b', 'c', 'a', 'b', ' ', '.', ',', '\\x ', '\\y ', '\\relax ', '~', '\\amp ', '\\hash ', '\\lb ', '\\rb ', '\\space ', '\\undefinedcs ', '\\pr ']
M_RARE = ['\\par ', '&', '\\cr ', '\\span ', '\\crr ', '\\O ', '\\noexpand ', '\\noexpand\\x ', '\\noexpand~', '\\iftrue ', '\\iffalse ', '\\fi ', '\\else ', '\\or ',
          '\\yfi ', '\\expandafter ', '\\the\\toks0 ', '\\string ', '\\unexpanded{a\\x}', '\\detokenize{a\\x}', '\\number 12 ', '\\csname x\\endcsname ', '\\endgroup ', '\\begingroup ']
M_DELIMITERS = ['.', ',', 'a', 'ab', 'aab', 'aba', '\\x ', '\\x\\y ', '\\relax ', ' ', '\\par ', 'a\\x ', '\\x a', '\\x\\x\\y ']


def m_text(r, depth, params, macros, rare):
    out = []
    for _ in range(r.randint(0, 6)):
        k = r.random()
        if k < 0.12 and depth > 0:
            out.append('{' + m_text(r, depth - 1, params, macros, rare) + '}')
        elif k < 0.32 and params:
            out.append('#%d' % r.randint(1, params))
        elif k < 0.55 and macros:
            out.append(m_call(r, depth, params, macros, rare))
        elif k < 0.55 + rare:
            out.append(r.choice(M_RARE))
        else:
            out.append(r.choice(M_ATOMS))
    return ''.join(out)


def m_call(r, depth, params, macros, rare):
    name, spec = r.choice(macros)
    out = [name + ' ']
    for delim in spec:
        arg = m_text(r, max(depth - 1, 0), params, macros, rare) if depth > 0 else r.choice(M_ATOMS)
        if delim is None:
            out.append('{' + arg + '}' if r.chance(0.6) else r.choice(['a', '\\x ', '\\relax ', ' a', '{}', '#1' if params else 'b']))
        else:
            out.append(arg + delim if r.chance(0.85) else arg)
    return ''.join(out)


def m_define(r, i, macros, rare):
    name = '\\m' + label(i)
    n = r.randint(0, 4)
    spec, head = [], ''
    for k in range(n):
        head += '#%d' % (k + 1)
        delim = r.choice(M_DELIMITERS) if r.chance(0.5) else None
        head += delim or ''
        spec.append(delim)
    tail = '#' if r.chance(0.05) else ''
    prefix = r.choice(['', '', '', '\\long', '\\long', '\\protected'])
    kind = r.choice(['\\def', '\\def', '\\def', '\\def', '\\edef', '\\gdef'])
    body = m_text(r, 2, n, macros, rare)
    macros.append((name, spec))
    return '%s%s%s %s%s{%s}\n' % (prefix, kind, name, head, tail, body)


def gen_macros(r):
    rare = r.choice([0.0, 0.02, 0.06, 0.15])
    outer = r.chance(0.3)
    macros, lines = [], [PREAMBLE, '\\tracingmacros=%d \\def\\O{plain}\\toks0={a\\x}\\def\\Z{\\noindent\\par}\n' % r.pick(0, 1, 1, 2)]
    for i in range(r.randint(10, 60)):
        lines.append(m_define(r, i, macros, rare))
        for _ in range(r.randint(0, 2)):
            c = m_call(r, 2, 0, macros, rare) if macros else ''
            if outer and r.chance(0.1):
                lines.append('\\outer\\def\\O{outer}\n')
            where = r.random()
            if where < 0.5:
                lines.append('\\setbox0\\hbox{' + c + '}\\Z\n')
            elif where < 0.7:
                lines.append(c + '\\Z\n')
            elif where < 0.85:
                lines.append('\\setbox0\\vbox{\\halign{##&##\\cr x' + c + '&y\\cr}}\\Z\n')
            elif where < 0.95:
                lines.append('\\edef\\e{' + c + '}\\Z\n')
            else:
                lines.append('\\iffalse ' + c + '\\fi\\Z\n')
    lines.append('\\end\n')
    return single(''.join(lines), r)


# ---------------------------------------------------------------------------------------------------------------------
# delims: delimiters that start like one another, with #{, spaces, \par and braces, called from every kind of list.
# ---------------------------------------------------------------------------------------------------------------------
D_PIECES = ['a', 'b', 'c', 'aa', 'ab', 'ba', 'aab', 'aba', 'abab', 'abc', 'abcab', ' ', 'a ', ' a', '\\x ', '\\y ', '\\x\\x ', '\\x\\y ', '\\par ', '\\par a', 'a\\par ', '~', '~~', '\\sp ', '\\lb ', '&']
D_ARGS = ['a', 'b', 'c', 'ab', 'aa', 'aab', 'aba', 'abab', 'abc', 'abd', ' ', '  ', '\\x ', '\\y ', '\\x\\y ', '\\par ', '~', '{a}', '{ab}', '{}', '{{a}}', '{a}{b}', '{\\par}', '{a\\x}', 'a{b}c',
          '\\sp ', '\\lb ', '\\rb ', '&', '\\amp ', '\\cr ', '\\noexpand\\x ', '$', '^', '_', '\\relax ', '\\iftrue ', '\\fi ', '\\hash ', '\\O ']


def gen_delims(r):
    d = Doc(r)
    d.add('\\tracingmacros=%d \\def\\O{}\n' % r.pick(1, 1, 2))
    if r.chance(0.2):
        d.add('\\outer\\def\\O{}\n')
    macros = []
    for i in range(r.randint(2, 7)):
        name = '\\m' + label(i)
        head, n, parts = '', r.randint(1, 3), []
        for k in range(n):
            head += '#%d' % (k + 1)
            if r.chance(0.8):
                de = r.choice(D_PIECES)
                head += de
                parts.append(de)
            else:
                parts.append(None)
        hb = r.chance(0.12)
        if hb:
            head += '#'
        body = r.some(['#%d' % (k + 1) for k in range(n)] + ['z', '\\x ', '{', '}', '{#1}'], 0, 4)
        if body.count('{') != body.count('}'):
            body = body.replace('{', '').replace('}', '')
        prefix = r.pick('', '', '\\long', '\\global', '\\protected', '\\long\\protected')
        d.add('%s\\def%s%s{%s}\n' % (prefix, name, head, body))
        macros.append((name, parts, hb))
    for _ in range(r.randint(5, 25)):
        name, parts, hb = r.choice(macros)
        call = name + ' '
        for de in parts:
            call += r.some(D_ARGS, 0, 4)
            if de is not None and r.chance(0.92):
                call += de
        if hb:
            call += '{h}'
        if r.chance(0.5):
            call += r.pick('\\relax', '.', ' ', '{}', '\\par', 'a')
        w = r.random()
        if w < 0.45:
            d.add(d.deliver(call, 0.15))
        elif w < 0.6:
            d.add(d.deliver('\\setbox0\\vbox{\\halign{#&#\\cr ' + call + '&y\\cr}}', 0.1))
        elif w < 0.75:
            d.add(d.deliver('\\setbox0\\hbox{' + call + '}', 0.1))
        elif w < 0.85:
            d.add(d.deliver('\\edef\\e{' + call + '}\\show\\e', 0.1))
        elif w < 0.95:
            a = r.some(D_ARGS, 1, 3)
            e = d.name()
            d.add('\\def%s#1#2{%s #1#2}%s{%s}{%s}\n' % (e, name, e, a, r.some(D_ARGS, 1, 3)))
        else:
            d.add(d.deliver('\\iffalse ' + call + '\\fi\\relax', 0.1))
        d.add('\\Zz\n' if r.chance(0.02) else '\\par\n')
    return single(d.text(), r)


# ---------------------------------------------------------------------------------------------------------------------
# outer: an \outer macro, or the \outer endtemplate, in every position, before and after the first use of \outer at all.
# ---------------------------------------------------------------------------------------------------------------------
OUTER_SITES = [
    '\\m{a\\O b}', '\\m\\O ', '\\d a\\O b.', '\\d {a\\O}b.', '\\d a.', '\\l\\O.', '\\l{\\O}.', '\\m{{\\O}}', '\\e #1{}', '\\iffalse \\O\\fi ', '\\iffalse a\\iftrue\\O\\fi b\\else c\\fi ',
    '\\ifcase 2 \\O\\or\\O\\or d\\fi ', '\\def\\t{a\\O b}', '\\def\\t#1\\O{a}', '\\def\\t\\O{a}', '\\edef\\t{a\\O b}', '\\edef\\t{\\noexpand\\O}', '\\toks0={a\\O}', '\\toks0=\\expandafter{\\O}',
    '\\detokenize{a\\O}', '\\edef\\t{\\unexpanded{a\\O}}', '\\edef\\t{\\detokenize{\\O}}', '\\message{a\\O}', '\\errhelp={a\\O}', '\\mark{a\\O}', '\\immediate\\write16{a\\O}', '\\special{\\O}', '\\csname a\\O\\endcsname ',
    '\\ifcsname a\\O\\endcsname\\fi ', '\\halign{\\O#\\cr a\\cr}', '\\halign{#\\O\\cr a\\cr}', '\\halign{#\\cr\\O\\cr}', '\\halign{#&#\\O&\\cr a&b\\cr}', '\\halign{\\m{\\O}#\\cr}',
    '\\string\\O ', '\\meaning\\O ', '\\ifx\\O\\O\\fi ', '\\ifx\\O a\\fi ', '\\ifdefined\\O\\fi ', '\\let\\t=\\O ', '\\futurelet\\t\\relax\\O ', '\\noexpand\\O ', '\\show\\O ', '\\expandafter\\relax\\O ',
    '\\uppercase{a\\O}', '\\scantokens{\\O}', '\\scantokens{\\m\\O}', '\\scantokens{\\def\\t{\\O}}', '\\pdfstrcmp{\\O}{a}', '\\expanded{\\O}', '\\m\\OO ', '\\d \\OO.', '\\iffalse\\OO\\fi ',
    '\\def\\t{\\OO}', '~', '\\m~', '\\d a~.', '\\iffalse~\\fi ', '\\def\\t{~}', '\\edef\\t{~}', '\\ifnum1<\\O 2\\fi ', '\\hskip 1pt\\O ', '\\count1=1\\O ', '\\hbox\\O ', '\\accent\\O ',
]


def gen_outer(r):
    d = Doc(r)
    d.add('\\def\\O{oo}\\def\\OO{pp}\\def\\m#1{[#1]}\\def\\d#1.{(#1)}\\long\\def\\l#1.{<#1>}\\def\\e#1#2{}\n')
    d.add('\\tracingmacros=%d \\tracingcommands=%d\n' % (r.pick(0, 1), r.pick(0, 0, 2)))
    sites = [r.choice(OUTER_SITES) for _ in range(r.randint(4, 20))]
    # A definition made before the meaning turns outer: the list holds the token, and only a capture loop's test can stop on it.
    early = []
    if r.chance(0.7):
        for s in sites[:len(sites) // 2]:
            e = d.name('\\E')
            early.append(e)
            d.add('\\def%s{%s}\n' % (e, s.replace('#', '##')))
    make = r.pick('\\outer\\def\\O{oo}', '\\outer\\long\\def\\O#1{oo}', '\\global\\outer\\def\\O{oo}', '\\outer\\def\\O#1.{oo}', '\\outer\\protected\\def\\O{oo}', '\\outer\\edef\\O{a}', '\\let\\O=\\OOO')
    d.add('\\outer\\def\\OOO{qq}\n' if 'OOO' in make else '')
    when = r.random()
    if when < 0.15:
        pass  # never: any_user_outer stays down.
    elif when < 0.3:
        d.add('\\begingroup' + make + '\\endgroup\n')
    else:
        d.add(make + '\n')
    if r.chance(0.3):
        d.add('\\outer\\def~{tt}\n')
    if r.chance(0.2):
        d.add('\\let\\OO=\\O\n')
    for e in early:
        d.add(e + '\\par\n')
    for s in sites[len(sites) // 2:]:
        d.add(d.deliver(s, 0.3))
        d.add('\\par\n')
    return single(d.text(), r)


# ---------------------------------------------------------------------------------------------------------------------
# align: & \span \cr in arguments and with align_state in every state, nesting, \noalign, \omit, \tabskip, unbalanced braces by \iffalse.
# ---------------------------------------------------------------------------------------------------------------------
A_PRE = ['#', '#', '#', '\\m#', '#\\m', '\\d#.', '\\d #', '#.', '#\\d', 'a#b', '{#}', '\\hbox{#}', '\\iffalse{\\fi#\\iffalse}\\fi', '\\tabskip=1pt #', '\\tabskip=0pt plus 1fil #', '\\x#', '#\\x', '\\span\\x #', '#\\span\\x',
         '\\relax#', '\\expandafter\\x #', '\\ifnum1<2 #\\fi', '\\if a#\\fi', '##', '\\O#', '\\ifx#\\fi', '\\scantokens{#}', '\\def\\t{}#', '\\z#', '\\noexpand\\x#', '\\lb #', '\\rb #', '#\\rb', '#\\lb',
         '\\mm{a}#', '\\halign{#\\cr a\\cr}#', '\\omit#', '\\vrule#', '#\\vrule', '\\iftrue#\\else\\fi', '\\global\\tabskip=2pt #', '\\the\\toks2 #', '\\span\\w #']
A_CELL = ['a', 'b', 'x y', '{a}', '{a&b}', '{a\\cr b}', '\\m{a}', '\\m{&}', '\\m&', '\\m\\cr ', '\\m\\span ', '\\m\\amp ', '\\m\\crr ', '\\d a&b.', '\\d a\\cr.', '\\d{&}.', '\\l a\\cr b.', '\\mm{a&}', '\\z ', '\\z a', '\\mm\\cr ',
          '\\iffalse{\\fi', '\\iffalse}\\fi', '{', '}', '\\lb ', '\\rb ', '\\omit ', '\\span ', '\\spn ', '\\noalign{\\kern1pt}', '\\noalign{\\m&}', '\\noalign\\lb\\kern1pt\\rb ', '\\noalign ', '\\relax ', '\\cr ', '\\crr ', '\\crcr ', '&', '\\amp ', '&&', '\\ifnum1<2 &\\fi ',
          '\\iftrue&\\fi', '\\iffalse&\\fi', '\\ifnum0<1\\cr\\fi', '\\expandafter\\m\\expandafter{\\x}', '\\if a\\cr\\fi', '\\if a&b\\fi', '\\ifx&\\cr\\fi ', '\\ifcat&\\cr\\fi', '\\valign{#\\cr a\\cr}', '\\hbox{\\valign{#\\cr a\\cr}}', '\\vbox{\\halign{#&#\\cr a&b\\cr}}', '\\halign{#\\cr a\\cr}',
          '\\hbox{\\halign{#\\cr}}', '\\par ', '\\setbox0\\hbox{&}', '\\setbox0\\hbox{\\cr}', '\\mark{a&b}', '\\message{a&\\cr}', '\\edef\\t{&\\cr}', '\\def\\t#1{&}\\t a', '\\def\\t#1&{}\\t a&', '\\def\\t#1\\cr{}\\t a\\cr ', '\\let\\t=&', '\\let\\t=\\cr ', '\\show\\t ', '\\showlists ', '\\showgroups ',
          '\\tabskip=3pt ', '\\O ', '\\detokenize{&}', '\\unexpanded{\\cr}', '\\number1&', '\\count1=1&', '\\hskip 1pt&', '\\hskip 1pt plus 1pt\\cr ', '\\ifnum1<2\\cr', '\\ifnum1<2&', '\\endgroup ', '\\begingroup ', '\\end ', '$', '$a$', '\\the\\toks2 ', '\\w ']


def gen_align(r):
    d = Doc(r)
    d.add('\\def\\m#1{[#1]}\\def\\mm#1{#1}\\def\\d#1.{(#1)}\\long\\def\\l#1.{<#1>}\\def\\z{\\ifnum0=`{\\fi}\\def\\O{oo}\\def\\w{}\\toks2={}\n')
    d.add('\\errorcontextlines=%d \\tracingcommands=%d \\tracingmacros=%d \\tracingalignascii=0 \\tracingonline=0\n' % (r.pick(0, 5, 20, 100), r.pick(0, 0, 1, 2), r.pick(0, 1, 2)))
    if r.chance(0.15):
        d.add('\\outer\\def\\O{oo}\n')
    if r.chance(0.3):
        d.add('\\everycr={%s}\n' % r.pick('\\noalign{\\kern1pt}', 'a', '\\relax', '\\m&', '\\mm{\\relax}', '\\omit', ''))
    if r.chance(0.2):
        d.add('\\toks2={%s}\n' % r.pick('&', '\\cr', '\\relax', '#', 'a&b\\cr'))
    if r.chance(0.2):
        d.add('\\def\\w{%s}\n' % r.pick('&', '\\cr', '\\omit', '\\span', '##', '\\noalign{}', 'a&b\\cr c&d\\cr'))
    for _ in range(r.randint(2, 8)):
        pre = '&'.join(r.choice(A_PRE) for _ in range(r.randint(1, 3)))
        if r.chance(0.2):
            pre += '&&' + r.choice(A_PRE)
        rows = []
        for _ in range(r.randint(0, 4)):
            rows.append(''.join(r.choice(A_CELL) if r.chance(0.7) else r.pick('p', 'q', '&', '&', '\\omit x') for _ in range(r.randint(0, 5))) + r.pick('\\cr', '\\cr', '\\cr', '\\crcr', '\\crr', ''))
        kind = r.pick('\\halign', '\\halign', '\\halign', '\\valign')
        spec = r.pick('', '', '', ' to 100pt', ' spread 10pt', ' to\\hsize', ' spread')
        body = '%s%s{%s\\cr %s}' % (kind, spec, pre, ''.join(rows))
        w = r.random()
        if w < 0.2:
            body = '\\hbox{%s}' % body
        elif w < 0.4:
            body = '\\setbox0\\vbox{%s}' % body
        elif w < 0.5:
            body = '$$%s$$' % body
        elif w < 0.55:
            body = '\\vbox{\\moveleft 1pt\\hbox{%s}}' % body
        d.add(d.deliver(body, 0.25))
        d.add('\\par\n' if r.chance(0.9) else '}\\par\n')
    return single(d.text(), r)


# ---------------------------------------------------------------------------------------------------------------------
# tables: alignments that mostly work: periodic preambles, \span, \omit, \noalign, and macros in cells and templates, read from lists at every align_state.
# ---------------------------------------------------------------------------------------------------------------------
TB_TEMPLATES = ['#', '#', '#', '\\hfil#', '#\\hfil', '\\hfil#\\hfil', '[#]', '\\u{a}#', '#\\u{b}', '\\strut#', '\\tabskip=1pt plus 1fil #', '\\tabskip=0pt #', '\\d x.#', '{#}', '\\ignorespaces#', '#\\unskip', '\\gg #', '#\\gg', '\\mt#', '$#$', '\\iftrue#\\fi', '\\vrule#', '\\hbox{#}', '\\vtop{#}', '\\span\\tp #', '\\span\\tq', '\\tn#', '\\u#', '\\d #.']
TB_CELLS = ['a', 'b', ' c ', '{d}', '\\u{e}', '\\d f.', '\\omit g', '\\omit', '\\gg ', '\\ee ', '\\hfil h', '\\multi 2 i', '\\span', '\\amp ', '\\span\\omit', '\\pc{j}', '\\pd k&', '\\pe l\\cr', '\\pf', '\\iffalse{\\fi\\iffalse}\\fi', '\\bg m\\eg', '\\vb', '\\def\\t#1{}\\t n', '\\dd', '\\ed', '\\tk', '\\sk{\\cr}', '\\sk&', '\\skd&.', '\\skd\\cr.', '\\mf{o}', '\\aftergroup\\x ', '\\ifx&\\cr x\\fi ', '\\ifcat&\\amp y\\fi ', '\\expandafter\\u\\expandafter{\\x}', '\\the\\toks2 ', '\\noexpand\\relax ', '\\number 1', '\\number1&', '\\hskip 1pt&', '\\kern1pt\\cr', '\\vrule width 1pt&', '\\vrule width 1pt\\cr', '\\vrule\\cr', '\\ifnum1<2&\\fi', '\\ifnum1<2 \\cr\\fi', '\\iftrue\\cr\\fi', '\\ifcase 1 a\\or&\\fi', '\\def\\s{a}\\s', '\\showlists ']
TB_MACROS = r'''\def\u#1{[#1]}\def\d#1.{(#1)}\let\bg={\let\eg=}\def\gg{{\ifnum0=`}\fi}\def\ee{\ifnum0=`{\fi}}\def\strut{\vrule height 8pt depth 3pt width 0pt}\def\mt{\relax}
\def\multi#1{\omit\span\multispan{#1}}\def\multispan#1{\ifnum#1>1 \omit\span\multispan{\numexpr#1-1\relax}\fi}\def\pc#1{\u{#1}&}\def\pd#1&{\u{#1}&}\def\pe#1\cr{\u{#1}\cr}\def\pf{&}\def\vb{\vbox{\halign{##\cr x\cr}}}
\def\dd{\def\t##1{a##1}\t b}\def\ed{\edef\t{a\x}}\def\tk{\toks2={c&d}}\def\sk#1{\iffalse#1\fi}\def\skd#1.{\iffalse#1\fi}\def\mf#1{\def\t{#1}\t}\def\tp{a}\def\tq{#}\def\tn{\noexpand\relax}\toks2={}
'''


def gen_tables(r):
    d = Doc(r)
    d.add(TB_MACROS)
    d.add('\\tracingalignascii=0 \\errorcontextlines=%d \\tracingcommands=%d \\tracingmacros=%d\n' % (r.pick(5, 100), r.pick(0, 0, 2), r.pick(0, 0, 1)))
    for _ in range(r.randint(1, 5)):
        n = r.randint(1, 3)
        pre = '&'.join(r.choice(TB_TEMPLATES) for _ in range(n))
        if r.chance(0.5):
            pre = r.pick('', '%s&' % r.choice(TB_TEMPLATES)) + '&' + '&'.join(r.choice(TB_TEMPLATES) for _ in range(r.randint(1, 2)))
        rows = ''
        for _ in range(r.randint(1, 6)):
            w = r.random()
            if w < 0.12:
                rows += '\\noalign{%s}' % r.pick('\\kern1pt', '\\u{a}', '\\hrule', '\\par', '\\sk&', '\\gg', '')
            elif w < 0.15:
                rows += '\\noalign\\bg\\kern 1pt\\eg '
            cells = [r.choice(TB_CELLS) if r.chance(0.5) else r.pick('p', 'q r', '', '{s}') for _ in range(r.randint(1, 8))]
            rows += '&'.join(cells) + r.pick('\\cr', '\\cr', '\\cr', '\\crcr', '\\cr\\crcr', '\\crr ', '\\cr\\noalign{}')
        body = '%s%s{%s\\cr %s}' % (r.pick('\\halign', '\\halign', '\\valign'), r.pick('', '', ' to 200pt', ' spread 5pt'), pre, rows)
        w = r.random()
        if w < 0.35:
            body = '\\setbox1\\vbox{%s}' % body if 'halign' in body[:8] else '\\setbox1\\hbox{%s}' % body
        elif w < 0.45:
            body = '\\halign{##&##\\cr\\vbox{%s}&x\\cr}' % body.replace('#', '##')
        elif w < 0.5:
            body = '$$%s$$' % body
        d.add(d.deliver(body, 0.3))
        d.add(r.pick('\\par\n', '\\par\\showlists\n', '\\showbox1 \n', '\n'))
    return single(d.text(), r)


# ---------------------------------------------------------------------------------------------------------------------
# stops: each capture loop, on each thing it stops for, in a body or an argument, at each kind of align_state, with something at once after that shows the input stack.
# ---------------------------------------------------------------------------------------------------------------------
ST_READERS = ['\\u ', '\\ul ', '\\d ', '\\dl ', '\\dab ', '\\dx ', '\\dp ', '\\dpl ', '\\dh ', '\\dd ', '\\ds ', '\\dq ', '\\ua ', '\\iffalse ', '\\ifcase 1 ', '\\def\\t ', '\\def\\t#1', '\\edef\\t ', '\\edef\\t#1', '\\toks0=', '\\detokenize ', '\\unexpanded ', '\\message ', '\\mark ', '\\expandafter\\u\\expandafter ', '\\immediate\\write16', '\\special ', '\\showtokens ', '\\lowercase ', '\\expanded ', '\\scantokens ', '\\pdfstrcmp{a}', '\\csname ', '\\ifcsname ', '\\number ', '\\ifnum ', '\\ifdim ', '\\count1=', '\\dimen1=', '\\hskip ', '\\vrule ', '\\let\\t=', '\\futurelet\\t\\x ', '\\afterassignment\\x ', '\\aftergroup ']
ST_TEXT = ['a', 'b', 'ab', 'aab', ' ', '\\x ', '\\y ', '\\relax ', '.', '#1', '#2', '#1', '#2', '{', '}', '{a}', '{}', '{#1}', '{a#1}', '{#1b}', '{{#1}}', '{#2}#1', '\\par ', '\\O ', '~', '&', '\\cr ', '\\span ', '\\amp ', '\\lb ', '\\rb ', '\\sp ', '\\noexpand\\x ', '\\noexpand ', '\\fi ', '\\else ', '\\iftrue ', '\\iffalse ', '\\p ', '\\undefinedone ', '\\BAD ', '\\hash ', '1', '2pt', '12', '1.5pt', '\\endcsname ', '\\the\\toks2 ', '\\number 3 ', '$', '^', '_', '\\showifs ', '\\pdfprimitive\\relax ', '\\protectedone ']
ST_ARGS = ['', 'a', 'ab', 'aab', '{a}', '{}', 'a{b}', '{a}b', '\\x ', '\\y ', ' ', '.', '\\par ', '\\O ', '~', '&', '\\cr ', '\\span ', '\\amp ', '\\sp ', '\\noexpand\\x ', '\\noexpand ', '\\fi ', '\\else ', '\\iftrue ', '\\p ', '\\undefinedone ', '\\BAD ', '#', '##', '1', '2pt', '\\lb ', '\\rb ', '\\endcsname ', '{\\par}', '{\\O}', '{&}', '{\\cr}', '{\\fi}', '{#}', '\\the\\toks2 ', '$', '^']
ST_AFTER = ['', '', '\\undefinedone ', '\\showlists ', '\\showtokens{z}', '\\showifs ', '\\errmessage{e}', '\\show\\t ', '\\relax ', '}', '\\fi ', '&', '\\cr ', '\\par ']
ST_MACROS = r'''\def\u#1{[#1]}\long\def\ul#1{<#1>}\def\d#1.{(#1)}\long\def\dl#1.{(#1)}\def\dab#1ab{(#1)}\def\dx#1\x{(#1)}\def\dp#1\par{(#1)}\long\def\dpl#1\par{(#1)}\def\dh#1#{(#1)}\def\dd#1.#2.{(#1#2)}\def\ds#1 {(#1)}\def\dq#1\x\y{(#1)}\def\ua#1a{(#1)}\protected\def\p{pp}\protected\def\protectedone{oo}\def\O{oo}\toks2={k\x}
'''


def gen_stops(r):
    d = Doc(r)
    d.add(ST_MACROS)
    d.add('\\errorcontextlines=%d \\tracingmacros=%d \\tracingcommands=%d \\tracingifs=%d\n' % (r.pick(5, 100), r.pick(0, 1, 1, 2), r.pick(0, 0, 1, 2), r.pick(0, 0, 1)))
    d.add('\\edef\\BAD{\\ifnum -1<\\pdfprimitive\\fi 2 \\fi}\n' if r.chance(0.1) else '\\def\\BAD{bad}\n')
    if r.chance(0.35):
        d.add('\\outer\\def\\O{oo}\n')
    if r.chance(0.1):
        d.add('\\outer\\def~{tt}\n')
    if r.chance(0.1):
        d.add('\\partokenname\\xpar\\let\\xpar=\\relax\n')
    # A group of many pieces, around capture.rs's GROUP_PIECES: text and arguments in turn, of which one may hold what stops the loop.
    if r.chance(0.3):
        e = d.name('\\M')
        pieces = ''.join(r.pick('', 'x', 'y{z}', '{}', ' ') + '#%d' % r.randint(1, 3) for _ in range(r.randint(1, 12)))
        reader = r.pick('\\u', '\\ul', '\\d', '\\dl', '\\detokenize', '\\message', '\\toks0=', '\\def\\t', '\\edef\\t', '\\expandafter\\u\\expandafter')
        d.add('\\def%s#1#2#3{%s{%s}%s%s}\n' % (e, reader, pieces, r.pick('', '.', ''), r.choice(ST_AFTER)))
        args = [r.pick('', '', 'a', 'a{b}c', '{}', '\\x ', r.choice(ST_ARGS)) for _ in range(3)]
        args = [a if a.count('{') == a.count('}') else '' for a in args]
        d.add('%s{%s}{%s}{%s}\n\\par\n' % (e, args[0], args[1], args[2]))
    # At most a few calls, so that the statistics at the end say something of each.
    for _ in range(r.randint(1, 6)):
        e = d.name('\\M')
        reader = r.choice(ST_READERS)
        text = r.some(ST_TEXT, 0, 5)
        # A brace that is not matched in the body is written the way that passes \def.
        if reader.endswith(' ') or reader.endswith('=') or reader.endswith('6'):
            w = r.random()
            if w < 0.7:
                text = '{' + text + '}'
        depth, fixed = 0, ''
        for c in re.findall(r'\\.|.', text):
            if c == '{':
                depth += 1
            elif c == '}':
                if depth == 0:
                    fixed += '\\iffalse{\\fi}'
                    continue
                depth -= 1
            fixed += c
        text = fixed + '}' * depth
        after = r.choice(ST_AFTER)
        tail = r.pick('', '', '.', 'ab', '\\x ', '\\par ', ' ', '{}', '\\x\\y ', '\\iffalse\\fi ', 'a', '\\fi ', '\\relax ')
        body = reader + text + tail + after
        body = body.replace('}\\iffalse{\\fi}', '}')
        params = r.pick('#1', '#1#2', '#1#2', '#1.#2.')
        fix_body = body if '#2' not in body or '#2' in params else body.replace('#2', '#1')
        args = [r.some(ST_ARGS, 0, 3) for _ in range(2)]
        args = [a if a.count('{') == a.count('}') else a.replace('{', '').replace('}', '') for a in args]
        if params == '#1':
            call = '%s{%s}' % (e, args[0])
        elif params == '#1#2':
            call = '%s{%s}{%s}' % (e, args[0], args[1])
        else:
            call = '%s%s.%s.' % (e, args[0], args[1])
        d.add('\\def%s%s{%s}\n' % (e, params, fix_body))
        k = r.random()
        if k < 0.45:
            d.add(call + '\n')
        elif k < 0.6:
            # In a cell, align_state 0 at its start, and in a template, where a u_template ends under the macro's eyes.
            d.add('\\setbox1\\vbox{\\halign{#&#\\cr %s&x\\cr}}\n' % call.replace('#', '##'))
        elif k < 0.7:
            d.add('\\setbox1\\vbox{\\halign{%s#&#\\cr a&x\\cr}}\n' % e if params == '#1' else '\\setbox1\\hbox{%s}\n' % call)
        elif k < 0.78:
            d.add('\\iffalse{\\fi %s\\iffalse}\\fi\n' % call)
        elif k < 0.86:
            d.add('\\iffalse}\\fi %s\\iffalse{\\fi\n' % call)
        elif k < 0.93:
            d.add('\\edef\\t{%s}\\show\\t\n' % call)
        else:
            d.add('\\def\\rr{%s}\\rr\n' % call.replace('#', '##'))
        d.add('\\par\n' if r.chance(0.7) else '')
    return single(d.text(), r)


# ---------------------------------------------------------------------------------------------------------------------
# deep: what the others do, with the input stack at or near its high water mark and near its end, where a level left out shows in "i" of the statistics or in the token an overflow comes at.
# ---------------------------------------------------------------------------------------------------------------------
DP_NUMBERS = ['1', '12', '1.5', '.5', '1,5', '16383', '2147483647', '12 ', '1\\relax', '`a', '"F']
DP_SITES = ['\\count1=\\numexpr 12 \\relax ', '\\count1=\\numexpr 12\\relax ', '\\dimen1=2 \\dimen3 ', '\\ifnum\\numexpr 1 \\relax<2 \\fi ', '\\ifnum 12 \\relax<2 \\fi ', '\\skip1=2 \\skip3 ', '\\ifdim 2 \\dimen3<1pt \\fi ', '\\ifnum 12<3 a\\fi ', '\\ifnum 12 <3 a\\fi ', '\\ifnum 12\\relax<3 a\\fi ', '\\ifnum 1a\\fi ', '\\ifnum 1\\x<2 \\fi ', '\\ifnum1<2 \\fi ', '\\the\\numexpr 12+3\\relax ', '\\the\\numexpr 12*3 ', '\\the\\numexpr 12 ', '\\the\\numexpr 12)', '\\the\\numexpr(12+3)*4\\relax ',
            '\\dimen1=1.5pt ', '\\dimen1=1.5pt\\relax ', '\\dimen1=1pt ', '\\dimen1=.5pt', '\\dimen1=1.5PT ', '\\dimen1=1.5truept ', '\\dimen1=1.5sp ', '\\dimen1=1.5mm ', '\\dimen1=1.5em ', '\\dimen1=1.5ex ', '\\dimen1=1.5px ', '\\dimen1=1 pt ', '\\dimen1=1.5\\p ', '\\dimen1=2\\dimen3 ', '\\dimen1=100000pt ', '\\dimen1=1.5p t', '\\dimen1=1.5p',
            '\\skip1=1pt plus 2pt minus 3pt ', '\\skip1=1.5pt plus 1fil ', '\\skip1=1pt plus 1fill minus 1filll ', '\\hskip 1.5pt\\relax ', '\\kern 1.5pt ', '\\vrule width 1.5pt height 2pt depth 3pt ', '\\hbox to 1.5pt{}', '\\hbox spread 1.5pt{}', '\\count1=12 ', '\\count1=12\\relax ', '\\count1=12\\x ', '\\advance\\count1 by 12 ', '\\number 12 ', '\\romannumeral 12 ',
            '\\csname abc\\endcsname ', '\\csname a\\x b\\endcsname ', '\\ifcsname abc\\endcsname\\fi ', '\\expandafter\\x\\y ', '\\expandafter\\u\\expandafter{\\x}', '\\noexpand\\x ', '\\iffalse a\\fi ', '\\iftrue a\\else b\\fi ', '\\ifcase 1 a\\or b\\or c\\fi ', '\\ifx\\x\\y\\fi ', '\\if aa\\fi ', '\\ifcat ab\\fi ', '\\ifdefined\\x\\fi ', '\\unless\\iftrue\\fi ',
            '\\u{a}', '\\u a', '\\u\\x ', '\\d a.', '\\d{a}.', '\\d{a}b.', '\\dab xaby', '\\dab xay ab', '\\dx a\\x ', '\\dh a{b}', '\\def\\t{a}', '\\def\\t#1#2{a#1}', '\\edef\\t{a\\x}', '\\toks0={a}', '\\the\\toks2 ', '\\edef\\t{\\the\\toks2 }', '\\detokenize{a\\x}', '\\unexpanded{a}', '\\message{a\\x}', '\\mark{a}', '\\scantokens{a}', '\\setbox0\\hbox{a}', '\\undefinedone ', '\\showtokens{a}', '\\showifs ', '\\show\\x ', '\\par ',
            '\\halign{#\\cr a\\cr}', '\\setbox0\\vbox{\\halign{#&#\\cr a&b\\cr}}', '\\setbox0\\vbox{\\halign{\\u#\\cr a\\cr}}', '\\lowercase{a}', '\\afterassignment\\x\\count1=1 ', '\\pdfstrcmp{a}{b}', '\\expanded{a}', '\\edef\\t{\\pdfstrcmp{a}{b}}']


def gen_deep(r):
    d = Doc(r, PREAMBLE)
    d.add(ST_MACROS)
    d.add('\\def\\p{\\dimen3 }\\dimen3=1pt \\errorcontextlines=%d \\tracingmacros=%d \\tracingcommands=%d\n' % (r.pick(3, 100), r.pick(0, 0, 1), r.pick(0, 0, 1)))
    if r.chance(0.25):
        d.add('\\outer\\def\\O{oo}\n')
    # One level a call: what follows each call stays to be read.  The second kind holds arguments too, and the third reads them on the way down.
    d.add('\\def\\recm{\\advance\\count9 by -1 \\ifnum\\count9>0 \\expandafter\\recn\\else\\snippet\\fi\\relax}\\def\\recn{\\recm\\relax}\n')
    d.add('\\def\\recp#1{\\advance\\count9 by -1 \\ifnum\\count9>0 \\expandafter\\recq\\else\\snippet\\fi{#1}\\relax}\\def\\recq#1{\\recp{#1}\\relax}\n')
    d.add('\\def\\recr#1{#1\\advance\\count9 by -1 \\ifnum\\count9>0 \\recr{#1}\\relax\\else\\snippet\\fi\\relax}\n')
    size = r.randint(200, 240) if r.chance(0.5) else None
    depth = 3
    for _ in range(r.randint(1, 8)):
        snippet = r.some(DP_SITES, 1, 3)
        # The depth goes up by a little, so each round is at a new high water mark, or down again.
        if size and r.chance(0.6):
            depth = r.randint(max(1, size // 2 - 8), size // 2 + 2) if r.chance(0.5) else r.randint(max(1, size - 8), size + 2)
        else:
            depth = depth + r.randint(1, 3) if r.chance(0.7) else r.randint(1, 30)
        d.add('\\def\\snippet{%s}\\count9=%d %s\n' % (snippet.replace('#', '##'), depth, r.pick('\\recm', '\\recm', '\\recp{}', '\\recp{a}', '\\recr{}', '\\recr{\\relax}')))
        d.add('\\par\n' if r.chance(0.5) else '')
    case = single(d.text(), r)
    if size:
        case.options = ['-cnf-line=stack_size=%d' % size]
    return case


# ---------------------------------------------------------------------------------------------------------------------
# files: \endlinechar mid-file, \scantokens, \endinput, a file that ends inside an argument, a definition or skipped text, \read, \everyeof, the warnings of \tracingnesting.
# ---------------------------------------------------------------------------------------------------------------------
F_LINES = ['a b c', '\\m{a', 'b}', '\\def\\t{', '}', '\\d a', 'b.', '\\iffalse', '\\fi', '\\iftrue', '\\else', '\\endinput', '\\endinput a', '\\endinput\\iffalse', '\\begingroup', '\\endgroup', '{', '\\input b', '\\input c', '\\input d ', '\\input e',
           '\\scantokens{a}', '\\scantokens{\\m{a}', '\\scantokens{\\def\\t{}', '\\scantokens{\\iffalse}', '\\scantokens{\\fi}', '\\scantokens{\\endinput}', '\\scantokens{a\\endinput b}',
           '\\endlinechar=-1 ', '\\endlinechar=13 ', '\\endlinechar=`A ', '\\endlinechar=`\\^^M', '\\endlinechar=255 ', '\\catcode`\\^^M=5 ', '\\catcode`\\^^M=13 ', '\\catcode`\\^^M=12 ', '\\catcode`\\^^M=10 ', '\\catcode`\\^^M=14 ', '\\catcode`\\^^M=9 ', '\\catcode`\\%=12 ', '\\catcode`\\%=14 ',
           '%comment', '   leading', 'trailing   ', '', '', '\\par', '\\relax', '\\everyeof={\\x}', '\\everyeof={\\noexpand}', '\\everyeof={}', '\\everyeof={\\m}', '\\everyeof={\\endinput}', '\\everyeof={\\fi}', '\\everyeof={a\\par}',
           '\\tracingscantokens=1 ', '\\tracingscantokens=0 ', '\\tracingnesting=1 ', '\\tracingnesting=2 ', '\\tracingnesting=0 ', '\\newlinechar=`A ', '\\newlinechar=`/ ', '\\newlinechar=-1 ', '\\message{\\the\\inputlineno}', '\\showifs ', '\\showgroups ',
           '\\openin3=c ', '\\read3 to\\t', '\\readline3 to\\t', '\\show\\t', '\\ifeof3 \\fi', '\\closein3 ', '\\immediate\\write16{\\the\\inputlineno}', '\\scantokens\\expandafter{\\the\\toks0}', '\\toks0={a\\par b}', '\\ifnum1<\\inputlineno\\fi', '\\ifnum1<2', '\\ifdim1pt<',
           '\\edef\\t{\\scantokens{a}}', '\\edef\\t{\\scantokens{\\noexpand}}', '\\edef\\t{a', '\\detokenize{', '\\halign{#\\cr', '\\cr}', '\\m', '\\d', '\\l',
           '\\edef\\t{\\input c }\\show\\t', '\\csname\\input c \\endcsname', '\\input\\input c ', '\\input b\\csname brandnew\\endcsname ', '\\input c\\relax', '\\message{\\input c }', '\\expandafter\\m\\input c ', '\\ifnum1<\\input c 2 \\fi', '\\input\\x c ', '\\input{c}', '\\font\\g=cmr\\csname nn\\endcsname10 ', '\\font\\g=cmr1\\input c ', '\\openin3=\\input c ', '\\openin3=b\\csname nm\\endcsname ', '\\immediate\\openout3=o\\number1 ', '\\immediate\\write3{a\\x}', '\\immediate\\closeout3 ', '\\immediate\\write16{\\input c }', '\\detokenize\\input c ',
           '\\expandafter\\m\\scantokens{a}', '\\expandafter\\show\\scantokens{a}', '\\expandafter\\m', '\\noexpand', '\\string', '\\input', '\\csname', '\\endcsname', '\\ifcsname']


def f_file(r):
    return '\n'.join(r.choice(F_LINES) for _ in range(r.randint(0, 8))) + r.pick('\n', '\n', '')


def gen_files(r):
    d = Doc(r)
    d.add('\\def\\m#1{[#1]}\\def\\d#1.{(#1)}\\long\\def\\l#1.{<#1>}\\toks0={a}\\def\\t{}\n')
    d.add('\\tracingmacros=%d \\tracingcommands=%d \\tracingscantokens=%d \\tracingnesting=%d\n' % (r.pick(0, 1), r.pick(0, 0, 1), r.pick(0, 0, 1), r.pick(0, 0, 1, 2)))
    for _ in range(r.randint(3, 25)):
        line = r.choice(F_LINES)
        if r.chance(0.3):
            d.add(d.deliver(line, 0.1))
        else:
            d.add(line + '\n')
    files = {'f.tex': d.text()}
    for name in 'bcde':
        files[name + '.tex'] = f_file(r)
    return Case(files, b'', r.pick('batchmode', 'batchmode', 'nonstopmode', 'scrollmode'))


# ---------------------------------------------------------------------------------------------------------------------
# cond: conditionals nested and skipped in every way, \csname with an undefined name, \expandafter chains, \romannumeral.
# ---------------------------------------------------------------------------------------------------------------------
C_TESTS = ['\\iftrue', '\\iffalse', '\\ifnum1<2 ', '\\ifnum2<1 ', '\\ifnum\\count1=0 ', '\\ifdim1pt<2pt ', '\\ifdim2pt<1pt ', '\\ifodd3 ', '\\ifodd2 ', '\\if aa', '\\if ab', '\\ifcat a1', '\\ifcat ab', '\\ifx aa', '\\ifx\\x\\y', '\\ifx\\x\\undefined', '\\ifx\\m\\m',
           '\\ifvmode', '\\ifhmode', '\\ifmmode', '\\ifinner', '\\ifvoid0 ', '\\ifhbox0 ', '\\ifvbox0 ', '\\ifeof3 ', '\\ifdefined\\x', '\\ifdefined\\undefinedname', '\\ifcsname x\\endcsname', '\\ifcsname nosuch\\endcsname', '\\ifcsname\\x\\endcsname', '\\ifcsname x\\relax\\endcsname',
           '\\iffontchar\\f`a ', '\\iffontchar\\f 300 ', '\\ifincsname', '\\ifpdfabsnum-1<2 ', '\\ifpdfabsdim-1pt<2pt ', '\\ifpdfprimitive\\relax', '\\ifpdfprimitive\\x', '\\ifpdfprimitive\\nosuchname', '\\unless\\iftrue', '\\unless\\iffalse', '\\unless\\ifnum1<2 ', '\\unless\\ifx aa',
           '\\unless\\ifcase', '\\unless', '\\ifcase 0 ', '\\ifcase 1 ', '\\ifcase 2 ', '\\ifcase-1 ', '\\ifnum\\pdfstrcmp{a}{a}=0 ', '\\ifnum\\pdfstrcmp{a}{b}<0 ', '\\if\\relax\\relax', '\\if\\noexpand\\x\\noexpand\\y', '\\ifcat\\noexpand~\\noexpand~', '\\ifx\\noexpand\\x\\x', '\\expandafter\\ifx\\csname x\\endcsname\\x',
           '\\expandafter\\ifx\\csname nosuch1\\endcsname\\relax', '\\if\\iftrue a\\fi a', '\\ifnum1<\\iftrue2\\fi ', '\\ifnum\\iftrue1\\fi<2 ', '\\ifnum1\\iftrue2\\fi<3 ', '\\if\\fi', '\\ifcat\\fi', '\\ifx\\fi\\else', '\\ifnum\\fi', '\\ifnum1<\\fi 2 ', '\\ifdim\\else', '\\ifodd\\or', '\\ifcase\\fi', '\\if\\else', '\\if a\\or',
           '\\pdfprimitive\\iftrue', '\\pdfprimitive\\iffalse', '\\expandafter\\pdfprimitive\\csname iftrue\\endcsname', '\\ifnum1<\\pdfprimitive\\fi 2 ', '\\if\\pdfprimitive\\fi a', '\\ifx\\pdfprimitive\\fi a', '\\ifnum\\numexpr1+1\\relax=2 ', '\\ifdim\\dimexpr1pt+1pt\\relax=2pt ', '\\ifnum\\numexpr 1 2 ', '\\ifnum\\numexpr(1+\\fi']
C_ATOMS = ['a', ' ', '\\x ', '\\relax ', '\\m{a}', '\\m\\fi ', '\\m\\else ', '\\m{\\fi}', '\\d a\\fi.', '\\e ', '\\f ', '\\g ', '\\number 1', '\\number\\count1 ', '\\romannumeral 5 ', '\\romannumeral 0 ', '\\romannumeral-1 ', '\\romannumeral\\count1 ', '\\expandafter\\expandafter\\expandafter\\m\\expandafter\\expandafter\\expandafter{\\expandafter\\x\\y}',
           '\\csname nosuch\\endcsname ', '\\csname x\\endcsname ', '\\csname\\endcsname ', '\\csname a\\relax\\endcsname ', '\\csname a\\fi\\endcsname ', '\\csname \\iftrue a\\fi\\endcsname ', '\\csname\\iffalse a\\fi b\\endcsname ', '\\endcsname ', '\\expandafter\\show\\csname b\\endcsname ', '\\expandafter\\ifx\\csname z\\endcsname\\relax\\fi ', '\\expandafter\\def\\csname y\\number\\count1\\endcsname{}',
           '\\expandafter\\relax\\x ', '\\expandafter\\relax\\relax ', '\\expandafter\\expandafter\\expandafter\\relax\\expandafter\\relax\\x ', '\\expandafter\\m\\expandafter{\\romannumeral 3 }', '\\expandafter\\m\\fi ', '\\expandafter\\m\\or ', '\\expandafter\\m\\else ', '\\expandafter\\fi\\iftrue ', '\\expandafter\\else\\fi ', '\\expandafter\\or\\fi ', '\\expandafter\\if\\relax ',
           '\\showifs ', '\\the\\currentiflevel ', '\\the\\currentiftype ', '\\the\\currentifbranch ', '\\noexpand\\fi ', '\\noexpand\\x ', '\\pdfprimitive\\relax ', '\\pdfprimitive\\x ', '\\pdfprimitive\\nosuchname ', '\\pdfprimitive a', '\\pdfprimitive\\fi ', '\\pdfprimitive\\else ', '\\pdfprimitive\\or ', '\\pdfstrcmp{a}{\\fi}', '\\pdfstrcmp{\\x}{b}', '\\advance\\count1 by 1 ', '\\begingroup ', '\\endgroup ', '\\par ', '{', '}']


def c_block(r, depth, rare):
    out = []
    for _ in range(r.randint(0, 4)):
        k = r.random()
        if k < 0.45 and depth > 0:
            test = r.choice(C_TESTS)
            parts = [test, c_block(r, depth - 1, rare)]
            if 'case' in test:
                for _ in range(r.randint(0, 3)):
                    parts += ['\\or ', c_block(r, depth - 1, rare)]
            if r.chance(0.5):
                parts += ['\\else ', c_block(r, depth - 1, rare)]
            if not r.chance(rare):
                parts.append('\\fi ')
            out.append(''.join(parts))
        elif k < 0.45 + rare:
            out.append(r.pick('\\fi ', '\\else ', '\\or ', '\\yfi ', '\\iftrue ', '\\iffalse '))
        else:
            out.append(r.choice(C_ATOMS))
    return ''.join(out)


# What a skip has to read in an argument: capture_skip reads that with no level, and makes one if it stops there.
C_SKIPPED = ['a', '\\fi ', '\\else ', '\\or ', '\\iftrue ', '\\iffalse\\fi ', '\\iftrue\\fi ', '\\ifcase 1 \\or\\fi ', '\\yfi ', '{', '}', '\\lb ', '\\rb ', '&', '\\cr ', '\\O ', '~', '\\noexpand\\fi ', '\\x ', '\\undefinedone ', '#', '\\par ', '\\m{a}', '\\expandafter\\fi ', '\\string\\fi ', '\\relax ', '\\BAD ']
C_SKIPPERS = ['\\iffalse', '\\iffalse', '\\ifcase 1 ', '\\ifcase 2 ', '\\ifnum1>2 ', '\\if ab', '\\ifx ab', '\\ifcase 0 a\\else', '\\iftrue a\\else', '\\unless\\iftrue', '\\ifdim 1pt>2pt ', '\\ifodd 2 ', '\\ifdefined\\nosuchname']


def c_skip_call(r, d):
    """A macro that skips its arguments, in the middle of what else it skips."""
    e = d.name('\\S')
    skip = r.choice(C_SKIPPERS)
    body = '%s %s#1%s%s#2%s\\fi %s' % (skip, r.pick('', 'p', '{', '\\iffalse\\fi '), r.pick('', 'q', '}', '\\iftrue\\fi '), r.pick('', '\\or ', '\\else '), r.pick('', 'r', '\\fi '), r.pick('', 'after', '\\undefinedone ', '\\showifs ', '\\showtokens{a}'))
    if body.count('{') != body.count('}'):
        body = body.replace('{', '').replace('}', '')
    args = ['{%s}' % r.some(C_SKIPPED, 0, 4) for _ in range(2)]
    args = [a if a.count('{') == a.count('}') else a.replace('{', '\\lb ').replace('}', '\\rb ') for a in args]
    return '\\def%s#1#2{%s}%s%s%s' % (e, body, e, args[0], args[1])


def gen_cond(r):
    d = Doc(r)
    d.add('\\def\\m#1{[#1]}\\def\\d#1.{(#1)}\\def\\e{\\iftrue}\\def\\f{\\fi}\\def\\g{\\else}\\count1=%d \\setbox0\\hbox{}\\def\\O{oo}\n' % r.randint(0, 3))
    # The token that §405 backs up with no control sequence, in a macro: PORTING section 15, "The token from no control sequence".
    d.add('\\edef\\BAD{\\ifnum -1<\\pdfprimitive\\fi 2 \\fi}\n' if r.chance(0.1) else '\\def\\BAD{}\n')
    if r.chance(0.25):
        d.add('\\outer\\def\\O{oo}\n')
    for _ in range(r.randint(0, 6)):
        d.add(c_skip_call(r, d) + '\n')
    d.add('\\tracingifs=%d \\tracingcommands=%d \\tracingmacros=%d \\tracingassigns=%d\n' % (r.pick(0, 0, 1), r.pick(0, 0, 1, 2), r.pick(0, 1), r.pick(0, 0, 1)))
    rare = r.choice([0.0, 0.05, 0.15])
    for _ in range(r.randint(4, 20)):
        b = c_block(r, 4, rare)
        w = r.random()
        if w < 0.1:
            b = '\\edef\\t{%s}\\show\\t' % b
        elif w < 0.2:
            b = '\\setbox1\\hbox{%s}' % b
        elif w < 0.25:
            b = '\\message{%s}' % b
        elif w < 0.3:
            b = '\\count2=\\number%s 1 ' % b
        elif w < 0.45:
            # What a loop does wrong to align_state shows only at a tab: so in a cell, with one after it.
            b = '\\setbox1\\vbox{\\halign{#&#\\cr %s&y\\cr}}' % b
        d.add(d.deliver(b, 0.15))
    return single(d.text(), r)


# ---------------------------------------------------------------------------------------------------------------------
# expand: \protected, \detokenize, \unexpanded, \noexpand, \the, \edef, \write, \message, \lowercase, \aftergroup, \afterassignment, \futurelet.
# ---------------------------------------------------------------------------------------------------------------------
E_ATOMS = ['a', ' ', '#', '##', '{a}', '{}', '\\x ', '\\y ', '\\p ', '\\lp ', '\\q ', '\\relax ', '~', '\\sp ', '\\noexpand\\x ', '\\noexpand\\p ', '\\noexpand~', '\\noexpand a', '\\noexpand\\undefinedone ', '\\noexpand\\noexpand ', '\\noexpand\\relax ', '\\noexpand{', '\\noexpand}', '\\noexpand#',
           '\\unexpanded{a\\x}', '\\unexpanded{\\p#}', '\\unexpanded\\expandafter{\\x}', '\\unexpanded\\relax{a}', '\\unexpanded a', '\\detokenize{a\\x #}', '\\detokenize{\\p~}', '\\detokenize\\expandafter{\\x}', '\\detokenize\\lb a\\rb ', '\\the\\toks0 ', '\\the\\toks2 ', '\\the\\count1 ', '\\the\\dimen1 ', '\\the\\skip1 ', '\\the\\font ', '\\the\\catcode`a ', '\\the\\x ', '\\the a',
           '\\number\\count1 ', '\\string\\x ', '\\string~', '\\string{', '\\string#', '\\meaning\\x ', '\\meaning\\p ', '\\meaning~', '\\meaning#', '\\meaning\\undefinedone ', '\\meaning\\sp ', '\\fontname\\f ', '\\pdfstrcmp{a\\x}{a}', '\\pdfstrcmp{\\unexpanded{\\x}}{\\noexpand\\x}', '\\pdfstrcmp{#}{##}', '\\expanded{a\\x}', '\\expanded{\\unexpanded{\\x}}', '\\expanded{\\p}', '\\expanded{#}',
           '\\expandafter\\noexpand\\x ', '\\expandafter\\x\\y ', '\\expandafter\\noexpand\\expandafter\\x\\y ', '\\csname p\\endcsname ', '\\expandafter\\noexpand\\csname x\\endcsname ', '\\pdfprimitive\\relax ', '\\pdfprimitive\\the\\count1 ', '\\pdfprimitive\\noexpand\\x ', '\\pdfprimitive\\unexpanded{a}', '\\ifx\\p\\p a\\fi ', '\\ifx\\noexpand\\x\\x b\\fi ', '\\if\\noexpand\\x\\relax c\\fi ', '\\ifcat\\noexpand\\p\\relax d\\fi ', '\\iftrue e\\else f\\fi ', '\\iffalse g\\fi ', '\\m{a}', '\\m\\p ', '\\m\\noexpand ', '\\m{\\noexpand\\x}', '\\m{#}', '\\m#', '\\n a', '\\n\\p ',
           '\\mark{a}', '\\topmark ', '\\botmark ', '\\marks1{a}', '\\topmarks1 ', '\\firstmarks 1 ', '\\pdfliteral{a}', '\\special{a}', '\\scantokens{a}', '\\scantokens{\\noexpand}', '\\tracingmacros=2 ', '\\tracingcommands=2 ']
E_SITES = ['\\edef\\t{%s}\\show\\t', '\\xdef\\t{%s}\\meaning\\t', '\\edef\\t#1{%s}\\show\\t', '\\edef\\t#1#2.{#2%s#1}\\show\\t', '\\def\\t{%s}\\show\\t', '\\def\\t#1{%s}\\t a', '\\long\\def\\t#1{%s}\\t{a\\par}', '\\protected\\def\\t{%s}\\edef\\u{\\t}\\show\\u', '\\message{%s}', '\\errhelp={%s}\\errmessage{%s}', '\\errmessage{%s}', '\\immediate\\write16{%s}', '\\write16{%s}\\shipout\\hbox{a}', '\\write-1{%s}\\shipout\\hbox{\\mark{b}}',
           '\\toks0={%s}\\showthe\\toks0 ', '\\toks0=\\expandafter{\\x %s}\\showthe\\toks0 ', '\\showtokens{%s}', '\\showtokens\\expandafter{%s}', '\\mark{%s}\\par\\showbox255 ', '\\shipout\\vbox{\\mark{%s}\\hbox{a}}\\message{\\topmark\\botmark}', '\\special{%s}', '\\pdfliteral{%s}\\shipout\\hbox{\\pdfliteral{%s}}', '\\lowercase{%s}', '\\uppercase{%s}', '\\setbox0\\hbox{%s}\\showbox0 ',
           '\\afterassignment\\x\\count1=1 %s', '\\aftergroup\\x{%s}', '\\begingroup\\aftergroup\\p\\aftergroup#\\aftergroup{%s\\endgroup', '\\futurelet\\t\\x %s\\show\\t', '\\let\\t=%s\\show\\t', '\\let\\t %s', '\\showtokens{%s}', '\\detokenize{%s}', '\\edef\\t{\\detokenize{%s}}\\show\\t', '\\edef\\t{\\unexpanded{%s}}\\show\\t', '\\edef\\t{\\the\\toks0 %s}\\show\\t', '\\scantokens{%s}', '\\expandafter\\show\\scantokens{%s}', '\\pdfmatch{%s}{a}', '\\pdfescapestring{%s}', '\\pdfstrcmp{%s}{%s}', '\\expanded{%s}', '\\edef\\t{\\expanded{%s}}\\show\\t', '\\vbox{\\write16{%s}}', '\\pdfmapline{}%s', '\\catcode`\\#=12 \\edef\\t{%s}\\show\\t \\catcode`\\#=6 ']


def gen_expand(r):
    d = Doc(r)
    d.add('\\def\\x{xx}\\def\\y{yy}\\protected\\def\\p{pp}\\protected\\long\\def\\lp#1{ll}\\def\\q#1{q#1}\\def\\m#1{[#1]}\\def\\n#1{\\noexpand#1}\\toks0={ta\\x #}\\toks2={\\p}\\count1=2 \\dimen1=3pt \\skip1=4pt plus 1fil \n')
    d.add('\\tracingmacros=%d \\tracingcommands=%d \\tracingassigns=%d \\tracingrestores=%d\n' % (r.pick(0, 1), r.pick(0, 0, 1), r.pick(0, 1), r.pick(0, 1)))
    if r.chance(0.2):
        d.add('\\outer\\def\\O{oo}\n')
    for _ in range(r.randint(4, 20)):
        site = r.choice(E_SITES)
        n = site.count('%s')
        body = site % tuple(r.some(E_ATOMS, 0, 5) for _ in range(n))
        if r.chance(0.15):
            body = '\\setbox0\\vbox{\\halign{#&#\\cr %s&y\\cr}}' % body
        d.add(d.deliver(body, 0.2))
    return single(d.text(), r)


# ---------------------------------------------------------------------------------------------------------------------
# scan: numbers, dimensions and keywords with every kind of end, from lists, at and below the stack's high water mark.
# ---------------------------------------------------------------------------------------------------------------------
S_NUMBERS = ['0', '1', '2', '12', '007', '2147483647', '2147483648', '214748364', '2147483649', '99999999999', '21474836470', '-1', '+1', '--1', '- 1', '"7F', '"FF ', "'17", "'8", '"G', '`a', '`\\a', '`{', '`}', '`~', '`\\relax', '`\\ab', '\\count1 ', '\\dimen1 ', '\\skip1 ', '\\muskip1 ', '\\x 1', '\\one', '\\one\\one', '\\ten', '\\c', '\\ch', '\\mc', '\\inputlineno ', '\\numexpr1+1\\relax', '\\numexpr 1 ', '\\numexpr(1+2)*3\\relax', '1\\iftrue2\\fi', '\\iftrue1\\fi', '1\\x 2', '1 2', '1\\sp 2', '', 'a', '.', ',', '\\relax', '\\pdfprimitive\\count1 ', '\\pdfprimitive\\relax', '\\romannumeral1 ', '\\number1 ']
S_ENDS = ['', ' ', '  ', '\\sp ', '\\sp\\sp ', '\\relax ', '\\x ', '\\relax\\relax ', '\\y ', 'a', 'b', '.', ',', '1', '0', '-', '+', '=', '<', '>', '{', '}', '\\lb ', '\\rb ', '&', '\\amp ', '\\cr ', '\\span ', '\\par ', '\\undefinedone ', '\\fi ', '\\else ', '\\or ', '~', '\\expandafter\\relax\\x ', '\\noexpand\\x ', '\\noexpand\\relax ', '\\the\\count1 ', '\\one', '\\endcsname ', '$', '^', '_', '#', '\\hash ', '\\O ', '\\iftrue a\\fi ', '\\iffalse a\\fi ', '\\pdfprimitive\\relax ', '\\show\\x ', '\\number1 ', 'p', 'pt', 't', 'plus', 'minus', 'P', 'e', 'x', 'm', 'u', 'mu', 'fil', 'true', 'em', 'ex', 'px', 'sp', 'cm', 'in', 'cc', 'dd', 'bp', 'nd', 'nc', 'pc']
S_UNITS = ['pt', 'pt', 'pt', 'PT', 'Pt', 'pT', 'p t', 'p\\x t', '\\x pt', ' pt', 'pt ', 'p', 't', 'sp', 'in', 'cm', 'mm', 'bp', 'pc', 'dd', 'cc', 'nd', 'nc', 'em', 'ex', 'px', 'mu', 'fil', 'fill', 'filll', 'filllll', 'fi', 'truept', 'true pt', 'TRUE PT', 'true sp', 'true em', 'true', '\\dimen1 ', '\\skip1 ', '\\count1 ', '\\fontdimen1\\f ', '\\wd0 ', '', '\\relax', 'p\\relax t', 'pt\\relax', 'pt\\sp ', '\\iftrue pt\\fi', 'p\\iftrue t\\fi', '\\one pt', 'pt\\x', '\\pt', '\\p@']
S_DECIMALS = ['1', '1.5', '1,5', '.5', ',5', '.', ',', '1.', '1,', '0.1', '12.25', '16383', '16383.99998', '16383.99999', '16384', '100000', '.00001', '0.000001', '1.123456789012345678901234', '1.5.5', '1.5,5', '-1.5', '+.5', '--2.5', '"10', "'10", '`a', '\\one', '\\one.5', '1\\one', '1.\\one', '1.\\x 5', '1\\x.5', '\\c', '\\c.5', '214748364', '2147483647', '2147483648', '99999.5', '1 .5', '1. 5', '\\count1 ', '\\dimen1 ', '-\\dimen1 ', '2\\dimen1 ', '2.5\\dimen1 ', '2.5\\wd0 ', '2\\skip1 ', '\\numexpr2\\relax', '1\\relax', '']
S_NUMSITES = ['\\count3=%s%s', '\\count3 %s%s', '\\advance\\count3 by %s%s', '\\multiply\\count3 %s%s', '\\divide\\count3 by %s%s', '\\number %s%s', '\\romannumeral %s%s', '\\ifnum %s%s<2 a\\fi ', '\\ifnum 2<%s%s a\\fi ', '\\ifnum %s%s=%s%s a\\else b\\fi ', '\\ifodd %s%s a\\fi ', '\\ifcase %s%s a\\or b\\fi ', '\\numexpr %s%s\\relax ', '\\the\\numexpr %s%s\\relax ', '\\the\\numexpr %s%s*2\\relax ', '\\the\\numexpr %s%s+%s%s\\relax ', '\\the\\numexpr(%s%s)/%s%s\\relax ', '\\the\\numexpr(%s%s)\\relax ', '\\showthe\\numexpr %s%s ', '\\count3=\\numexpr %s%s ', '\\the\\dimexpr 1pt*%s%s\\relax ', '\\catcode`\\a=%s%s\\catcode`\\a=11 ', '\\char %s%s', '\\penalty %s%s', '\\hyphenchar\\f=%s%s', '\\count%s%s=1 ', '\\box %s%s', '\\setbox %s%s\\hbox{}', '\\parshape %s%s 1pt 2pt ', '\\font\\g=cmr10 scaled %s%s ', '\\fontdimen %s%s\\f=1pt ', '\\the\\fontdimen %s%s\\f ', '\\pdfstrcmp{\\number %s%s}{1}', '\\csname\\number %s%s\\endcsname ', '\\edef\\t{\\number %s%s}\\show\\t', '\\edef\\t{\\ifnum %s%s<2 a\\fi}\\show\\t', '\\message{\\number %s%s}', '\\hbox{\\count3=%s%s}', '\\setbox0\\hbox{\\ifnum %s%s<3 \\fi}']
S_DIMSITES = ['\\dimen3=%s%s%s', '\\dimen3 %s%s%s', '\\advance\\dimen3 by %s%s%s', '\\hskip %s%s%s', '\\vskip %s%s%s', '\\kern %s%s%s', '\\skip3=%s%s%s', '\\skip3=1pt plus %s%s%s', '\\skip3=1pt minus %s%s%s', '\\skip3=%s%s%s plus %s%s%s minus %s%s%s', '\\hskip 1pt plus %s%s%s', '\\hskip %s%s%s plus 2pt', '\\ifdim %s%s%s<2pt a\\fi ', '\\ifdim 2pt<%s%s%s a\\fi ', '\\the\\dimexpr %s%s%s\\relax ', '\\the\\dimexpr %s%s%s+%s%s%s\\relax ', '\\the\\dimexpr(%s%s%s)*2\\relax ', '\\the\\glueexpr %s%s%s plus 1pt\\relax ', '\\showthe\\dimexpr %s%s%s ', '\\hbox to %s%s%s{}', '\\hbox spread %s%s%s{}', '\\vbox to %s%s%s{}', '\\vrule width %s%s%s', '\\vrule height %s%s%s depth %s%s%s width 1pt', '\\hrule height %s%s%s', '\\raise %s%s%s\\hbox{}', '\\moveleft %s%s%s\\hbox{}', '\\mskip %s%s%s', '\\mkern %s%s%s', '\\muskip3=%s%s%s', '\\hsize=%s%s%s', '\\wd0=%s%s%s', '\\fontdimen2\\f=%s%s%s', '\\parshape 1 %s%s%s 100pt ', '\\parshape 1 0pt %s%s%s', '\\pdfpagewidth=%s%s%s', '\\font\\g=cmr10 at %s%s%s', '\\pdfsavepos %s%s%s', '\\edef\\t{\\the\\dimexpr %s%s%s\\relax}', '\\edef\\t{\\ifdim %s%s%s<2pt a\\fi}', '\\setbox0\\hbox{\\ifdim %s%s%s<3pt \\fi}', '$\\mskip %s%s%s$', '\\pdfliteral direct{a}%s%s%s', '\\vadjust{\\kern %s%s%s}']
S_KEYWORDS = ['\\hbox to 1pt{}', '\\hbox TO 1pt{}', '\\hbox t\\x o 1pt{}', '\\hbox \\x to 1pt{}', '\\hbox tx{}', '\\hbox spread1pt{}', '\\hbox s{}', '\\hbox sp{}', '\\vrule width1pt\\relax', '\\vrule w', '\\vrule wi', '\\vrule height1pt width', '\\vrule depth', '\\vrule depth\\relax', '\\hrule w 1pt', '\\hskip 1pt plus', '\\hskip 1pt plu', '\\hskip 1pt pl', '\\hskip 1pt p', '\\hskip 1pt plus 1pt minus', '\\hskip 1pt minus 1pt plus 1pt', '\\hskip 1pt m', '\\font\\g=cmr10 s', '\\font\\g=cmr10 at', '\\font\\g=cmr10 scaled', '\\font\\g=cmr10 a', '\\vsplit 0 to', '\\vsplit 0 t', '\\advance\\count1 by1', '\\advance\\count1 b', '\\advance\\count1 BY', '\\read3 to\\t', '\\read3 t', '\\read3 \\t', '\\pdfliteral direct{}', '\\pdfliteral d{}', '\\pdfximage{a}', '\\pdfobj reserveobjnum', '\\pdfobj r']


def s_number(r):
    return r.choice(S_NUMBERS), r.choice(S_ENDS)


def gen_scan(r):
    d = Doc(r)
    d.add('\\def\\one{1}\\def\\ten{10}\\def\\pt{pt}\\dimen255=1pt \\let\\p@=\\dimen255 \\chardef\\c=5 \\chardef\\ch=255 \\mathchardef\\mc=300 \\count1=7 \\dimen1=2.5pt \\skip1=3pt plus 1fil \\muskip1=4mu \\setbox0\\hbox{\\vrule width 3pt}\\def\\O{oo}\\def\\t{}\n')
    d.add('\\tracingcommands=%d \\tracingmacros=%d \\tracingassigns=%d \\errorcontextlines=%d\n' % (r.pick(0, 0, 1), r.pick(0, 0, 1), r.pick(0, 0, 1), r.pick(5, 20, 100)))
    if r.chance(0.1):
        d.add('\\outer\\def\\O{oo}\n')
    # A deep level first, so that the high water mark of the stack stands above what follows, or not.
    if r.chance(0.5):
        deep = '\\def\\deep#1{\\ifnum#1>0 \\expandafter\\deep\\expandafter{\\number\\numexpr#1-1\\expandafter\\expandafter\\expandafter}\\expandafter\\relax\\fi}\\deep{%d}\\relax\n'
        d.add(deep % r.randint(1, 6))
    for _ in range(r.randint(8, 30)):
        w = r.random()
        if w < 0.4:
            site = r.choice(S_NUMSITES)
            body = site % tuple(x for _ in range(site.count('%s') // 2) for x in s_number(r))
        elif w < 0.8:
            site = r.choice(S_DIMSITES)
            body = site % tuple(x for _ in range(site.count('%s') // 3) for x in (r.choice(S_DECIMALS), r.choice(S_UNITS), r.choice(S_ENDS)))
        else:
            body = r.choice(S_KEYWORDS) + r.choice(S_ENDS)
        # Most from a macro's body and arguments: what the fast paths read.
        if r.chance(0.3):
            e = d.name()
            a, b = r.choice(S_DECIMALS), r.choice(S_UNITS)
            d.add('\\def%s#1#2#3{%s}%s{%s}{%s}{%s}\n' % (e, r.pick('\\hskip #1#2#3', '\\dimen3=#1#2#3', '\\ifdim #1#2#3<1pt \\fi', '\\the\\dimexpr #1#2#3\\relax', '\\count3=#1#3', '\\ifnum #1#3<1 \\fi', '\\the\\numexpr #1#3+#1#3\\relax'), e, a, b, r.choice(S_ENDS)))
        else:
            d.add(d.deliver(body, 0.15))
        if r.chance(0.3):
            d.add('\\showthe\\count3 \\showthe\\dimen3 \\showthe\\skip3 \\par\n')
    return single(d.text(), r)


# ---------------------------------------------------------------------------------------------------------------------
# active: active characters in every role, category codes on the fly, what \par and the end of a line turn into.
# ---------------------------------------------------------------------------------------------------------------------
V_ATOMS = ['a', '~', '!', '^^M', '^^41', '^^7f', '^^ff', '^^80', '^^?', '^^@', '^^a', '^^ab', '^^g', '^', '^^', '_', '$', '&', '#', '{', '}', '\\ ', '\\^^M', '\\^^41', '\\^^?', '\\^^ab', '\\a^^62', '\\ab^^63', '\\^', '\\~', '\\!', ' ', '  ', '\\par ', '\\x ', '\\m ', '\\d ', '\\l ']
V_SET = ['\\catcode`\\!=13 ', '\\catcode`\\!=12 ', '\\catcode`\\!=11 ', '\\catcode`\\!=1 ', '\\catcode`\\!=2 ', '\\catcode`\\!=4 ', '\\catcode`\\!=6 ', '\\catcode`\\!=7 ', '\\catcode`\\!=10 ', '\\catcode`\\!=9 ', '\\catcode`\\!=14 ', '\\catcode`\\!=15 ', '\\catcode`\\!=5 ', '\\catcode`\\a=13 ', '\\catcode`\\a=11 ', '\\catcode`\\a=12 ', '\\catcode`\\ =12 ', '\\catcode`\\ =10 ', '\\catcode`\\ =13 ', '\\catcode`\\^^M=13 ', '\\catcode`\\^^M=5 ', '\\catcode`\\^^A=15 ', '\\catcode`\\^^?=12 ', '\\catcode`\\^=12 ', '\\catcode`\\^=7 ',
         '\\def!{ee}', '\\def!#1{[#1]}', '\\def!#1!{(#1)}', '\\let!=\\par ', '\\let!=\\lb ', '\\let!=\\rb ', '\\let!=&', '\\let!=\\cr ', '\\let!=\\relax ', '\\let!=a', '\\let!=!', '\\let!=\\undefinedone ', '\\outer\\def!{oo}', '\\long\\def!#1{<#1>}', '\\protected\\def!{pp}', '\\def~{tt}', '\\def~#1{[#1]}', '\\let~=\\par ', '\\let~=\\lb ', '\\let~=\\rb ', '\\let~=\\fi ', '\\let~=\\iftrue ', '\\let~=\\amp ', '\\let~=\\hash ', '\\let~=\\sp ',
         '\\begingroup\\lccode`\\~=`\\!\\lowercase{\\endgroup\\def~{ll}}', '\\begingroup\\uccode`\\a=`\\~\\uppercase{\\endgroup\\def a{uu}}', '\\lccode`\\a=`\\!\\lowercase{\\show a}', '\\uccode`\\~=`\\a \\uppercase{\\show~}', '\\let\\par=\\relax ', '\\def\\par{\\relax}', '\\long\\def\\par#1{[#1]}', '\\let\\par=\\undefinedone ', '\\let\\par=\\endgraf ', '\\partokenname\\foo ', '\\partokenname~', '\\partokenname\\par ', '\\partokencontext=1 ', '\\partokencontext=2 ', '\\partokencontext=0 ', '\\let\\foo=\\relax ', '\\def\\foo{ff}', '\\endlinechar=`! ', '\\endlinechar=-1 ', '\\endlinechar=13 ', '\\escapechar=`! ', '\\escapechar=-1 ', '\\escapechar=92 ', '\\newlinechar=`a ', '\\newlinechar=-1 ']


def gen_active(r):
    d = Doc(r)
    d.add('\\def\\m#1{[#1]}\\def\\d#1.{(#1)}\\long\\def\\l#1.{<#1>}\\let\\endgraf=\\par\n')
    d.add('\\tracingmacros=%d \\tracingcommands=%d\n' % (r.pick(0, 1), r.pick(0, 0, 1)))
    for _ in range(r.randint(5, 30)):
        if r.chance(0.3):
            d.add(r.choice(V_SET) + '\n')
        line = r.some(V_ATOMS, 0, 6)
        w = r.random()
        if w < 0.4:
            d.add(line + '\n')
        elif w < 0.6:
            d.add('\\m ' + line + '\n')
        elif w < 0.75:
            d.add('\\d ' + line + '.\n')
        elif w < 0.85:
            d.add(d.deliver(line, 0.1))
        elif w < 0.9:
            d.add('\\edef\\t{' + line + '}\\show\\t\n')
        elif w < 0.95:
            d.add('\\iffalse ' + line + '\\fi\n')
        else:
            d.add('\\scantokens{' + line + '}\n')
    return single(d.text(), r)


# ---------------------------------------------------------------------------------------------------------------------
# trace: every \tracing and \show on what the others do.
# ---------------------------------------------------------------------------------------------------------------------
T_SETTINGS = ['\\showstream=%d ', '\\tracingcommands=%d ', '\\tracingmacros=%d ', '\\tracingifs=%d ', '\\tracingscantokens=%d ', '\\tracingassigns=%d ', '\\tracingnesting=%d ', '\\tracinggroups=%d ', '\\tracingrestores=%d ', '\\tracingparagraphs=%d ', '\\tracingpages=%d ', '\\tracingoutput=%d ', '\\tracinglostchars=%d ', '\\tracingonline=%d ', '\\showboxbreadth=%d ', '\\showboxdepth=%d ', '\\errorcontextlines=%d ', '\\tracingstacklevels=%d ', '\\tracingparagraphs=%d ', '\\pdftracingfonts=%d ']
T_SHOW = ['\\show\\x ', '\\show\\m ', '\\show\\undefinedone ', '\\show a', '\\show~', '\\show{', '\\show\\show ', '\\show\\fi ', '\\show\\relax ', '\\showthe\\count1 ', '\\showthe\\toks0 ', '\\showthe\\font ', '\\showthe\\lastpenalty ', '\\showlists ', '\\showbox0 ', '\\showbox255 ', '\\showifs ', '\\showgroups ', '\\showtokens{a\\x}', '\\showtokens\\expandafter{\\x}', '\\showtokens{\\showtokens}', '\\message{\\meaning\\m}', '\\message{\\the\\currentgrouplevel}', '\\message{\\the\\currentgrouptype}', '\\message{\\the\\currentiflevel}', '\\message{\\the\\interactionmode}', '\\interactionmode=0 ', '\\interactionmode=1 ', '\\interactionmode=2 ', '\\interactionmode=3 ', '\\interactionmode=4 ', '\\batchmode ', '\\nonstopmode ', '\\scrollmode ', '\\errorstopmode ']
T_DO = ['{', '}', '\\begingroup ', '\\endgroup ', '\\bgroup ', '\\egroup ', '\\hbox{a}', '\\setbox0\\hbox{a}', '\\setbox0\\vbox{a\\par}', '\\vbox{\\showlists}', '\\hbox{\\showgroups}', '$$\\showlists$$', '$a\\showlists$', '\\m{a}', '\\m\\x ', '\\d a.', '\\d{a}.', '\\m{a}\\par ', '\\iftrue ', '\\iffalse\\else ', '\\fi ', '\\else ', '\\ifnum1<2 \\showifs\\fi ', '\\ifcase1 \\or\\showifs\\fi ', '\\count1=5 ', '{\\count1=6 }', '\\global\\count1=7 ', '\\def\\t{}', '{\\def\\t{a}}', '\\gdef\\t{b}', '\\let\\t=\\x ', '\\toks0={b}', '\\skip1=1pt ', '\\dimen1=1pt ', '\\font\\g=cmr10 ', '\\hsize=100pt ', '\\catcode`\\!=12 ', '\\lccode`\\a=`\\b ', '\\parshape 1 0pt 100pt ', '\\interlinepenalties 1 1 ', 'abc def ghi\\par ', '\\vskip 1pt ', '\\penalty-10000 ', '\\mark{m}', '\\vadjust{a}', '\\insert100{a}', '\\noindent ', '\\indent ', '\\par ', '\\shipout\\hbox{a}', '\\output={\\shipout\\box255 }', '\\everypar={\\x}', '\\everyhbox={\\x}', '\\everyvbox={\\relax}', '\\afterassignment\\x\\count1=1 ', '\\aftergroup\\x ', '\\the\\toks0 ', '\\errmessage{e}', '\\undefinedone ', '\\halign{#\\cr a\\cr}', '\\scantokens{\\x}', '\\input b ', '\\pdfliteral{a}']


def gen_trace(r):
    d = Doc(r)
    d.add('\\def\\x{xx}\\def\\m#1{[#1]}\\def\\d#1.{(#1)}\\toks0={a}\\count1=1 \\setbox0\\hbox{}\\def\\t{}\n')
    for s in T_SETTINGS:
        if r.chance(0.3):
            d.add(s % r.pick(0, 1, 1, 2, 3, 10) + '\n')
    pool = T_SHOW + T_DO
    for _ in range(r.randint(5, 30)):
        if r.chance(0.15):
            d.add(r.choice(T_SETTINGS) % r.pick(0, 1, 1, 2, 3) + '\n')
        body = r.some(pool, 1, 5)
        d.add(d.deliver(body, 0.3))
    b = r.some(T_DO, 0, 3)
    return Case({'f.tex': d.text(), 'b.tex': b + '\n'}, b'', r.pick('batchmode', 'nonstopmode', 'scrollmode', 'scrollmode'))


# ---------------------------------------------------------------------------------------------------------------------
# interact: \errorstopmode with the terminal at its end or typing, deletion, \errmessage, \errhelp, runaways.
# ---------------------------------------------------------------------------------------------------------------------
I_ERRORS = ['\\undefinedone ', '\\m\\par ', '\\m}', '}', '\\d a\\par ', '\\errmessage{e}', '\\errmessage{a\\x}', '\\errhelp={h1^^Jh2}\\errmessage{e}', '\\errhelp={}\\errmessage{e}', '\\errorstopmode\\undefinedone ', '\\scrollmode\\undefinedone ', '\\batchmode\\undefinedone ', '\\nonstopmode\\undefinedone ', '\\interactionmode=2 \\undefinedone ', '\\interactionmode=0 \\errmessage{e}', '\\interactionmode=3 \\undefinedone ', '\\interactionmode=5 ', '\\interactionmode=-1 ', '\\hskip ', '\\count1=x', '\\ifnum1a\\fi ', '\\def\\t#2{}', '\\def\\t{#}', '\\def\\t#1#3{}', '\\def', '\\let', '\\m{\\O}', '\\iffalse\\O\\fi ', '\\def\\t{\\O}', '\\fi ', '\\else ', '\\or ', '\\ifx ', '\\csname\\relax\\endcsname ', '\\endcsname ', '\\unless a', '\\vrule\\par\\hrule ', '\\hbox\\par ', '\\setbox', '$', '&', '\\cr ', '\\span ', '#', '^', '_', '\\end ', '\\dump ', '\\begingroup\\end ', '\\read16 to\\t', '\\read3 to\\t', '\\immediate\\write16{w}', '\\message{m}', '\\show\\x ', '\\showthe\\count1 ', '\\showlists ', '\\showtokens{a}', '\\showifs ', '\\showgroups ', '\\input nosuchfile ', '\\openin3=nosuchfile ', '\\font\\g=nosuchfont ', '\\expandafter\\m\\expandafter{\\undefinedone}', '\\edef\\t{\\undefinedone}', '\\message{\\undefinedone}', '\\halign{#\\cr a&b\\cr}', '\\halign{\\undefinedone#\\cr a\\cr}', '\\errorcontextlines=-1 ', '\\errorcontextlines=0 ', '\\errorcontextlines=100 ', '\\tracingonline=1 ', '\\tracingmacros=1 ', '\\errhelp={}', '\\global\\x ', '\\long\\x ', '\\outer\\def\\O{}', '\\protected\\x ']
I_ERRORS += ['\\pausing=1 ', '\\pausing=0 ', '\\endlinechar=-1 ', '\\endlinechar=13 ', '\\read-1 to\\t', '\\readline16 to\\t', '\\readline-1 to\\t', '\\read3 to\\t\\show\\t ', '\\read16 to\\t\\show\\t ', '\\endinput ', '\\input f ', '\\scrollmode\\read16 to\\t', '\\nonstopmode\\read16 to\\t', '\\tracingonline=-1 ', '\\m\\O ', '\\m{\\par}', '\\d\\O.']
I_ANSWERS = [b'\n', b'\n', b'\n', b'1\n', b'2\n', b'3\n', b'5\n', b'9\n', b'10\n', b'99\n', b'100\n', b'h\n', b'H\n', b'I\\relax\n', b'I\n', b'Iabc\n', b'I}\n', b'I{\n', b'I\\undefinedtwo\n', b'I\\fi\n', b'q\n', b'r\n', b's\n', b'Q\n', b'R\n', b'S\n', b'e\n', b'x\n', b'X\n', b'D\n', b'?\n', b'z\n', b'\\relax\n', b'\\end\n', b'*\\end\n', b'\n\n', b'a b c\n', b'\\undefinedthree\n']


def gen_interact(r):
    d = Doc(r)
    d.add('\\def\\m#1{[#1]}\\def\\d#1.{(#1)}\\def\\x{xx}\\def\\O{oo}\\toks0={a}\\count1=1 \\def\\t{}\n')
    if r.chance(0.3):
        d.add('\\outer\\def\\O{oo}\n')
    if r.chance(0.2):
        d.add('\\tracingonline=1 \n')
    start = r.pick('errorstopmode', 'errorstopmode', 'errorstopmode', 'scrollmode', 'nonstopmode', 'batchmode')
    for _ in range(r.randint(1, 20)):
        d.add(d.deliver(r.some(I_ERRORS, 1, 3), 0.3))
    # At the end of its input the terminal is a fatal error at the first prompt; so half have one.
    answers = b''.join(r.choice(I_ANSWERS) for _ in range(r.randint(0, 25))) if r.chance(0.7) else b''
    return Case({'f.tex': d.text()}, answers, start)


# ---------------------------------------------------------------------------------------------------------------------
# limits: long tokens, long lines, large arguments across the heap's size classes, and the stacks to their ends.
# ---------------------------------------------------------------------------------------------------------------------
def l_sizes(r):
    # The exact classes end at 64 tokens, and each one above is 9/8 of the last: toks.rs, CLASS_CAP.
    cap, caps = 64, [0, 1, 2, 3, 62, 63, 64]
    while cap < 2000000:
        caps.append(cap)
        cap = cap + cap // 8 + 1
    c = r.choice(caps)
    return max(0, c + r.randint(-2, 2))


def gen_limits(r):
    d = Doc(r)
    k = r.randint(0, 15)
    d.add('\\tracingmacros=%d \\tracingonline=0 \\errorcontextlines=%d\n' % (r.pick(0, 0, 1) if k > 4 else 0, r.pick(3, 20)))
    d.add('\\def\\m#1{[#1]}\\def\\mm#1#2{#1#2#1}\\def\\d#1.{(#1)}\\long\\def\\l#1.{<#1>}\\def\\e#1{\\message{\\pdfmdfivesum{\\detokenize{#1}}}}\\def\\ee#1.{\\message{\\pdfmdfivesum{\\detokenize{#1}}}}\n')
    if k == 0:
        # Long names, and the name that a \csname makes, crossing what error_line and half_error_line show.
        for _ in range(r.randint(1, 5)):
            n = r.choice([20, 40, 41, 42, 43, 79, 80, 81, 200, 1000, 70000])
            name = 'a' * n
            d.add('\\expandafter\\def\\csname %s\\endcsname#1.{#1}\n' % name)
            d.add('\\%s b\\undefinedone .\n' % name)
            d.add('\\def\\t{\\%s}\\expandafter\\show\\t\n' % name)
            d.add('\\%sb\n' % name)
            d.add('\\message{\\string\\%s}\\expandafter\\m\\csname %sc\\endcsname\n' % (name, name))
    elif k == 1:
        # Long lines, in the file and from \scantokens, up to buf_size.
        n = r.choice([100, 1000, 30000, 199990, 199995, 199998, 199999, 200000, 200001, 200002, 210000])
        d.add(r.pick('', '\\m{', '\\undefinedone ', '\\d ') + 'a' * n + r.pick('', '}', '.') + '\n')
        d.add('\\undefinedtwo\n')
        # A line that leaves first at buf_size, or next to it, with a name to go there.  The line's place in the buffer depends on what stands before it, which is little here.
        body = r.pick('\\csname a\\endcsname', '\\csname ab\\endcsname', '\\csname\\endcsname', '\\ifcsname a\\endcsname\\fi', '\\ifcsname abc\\endcsname\\fi', '\\message{a}', '\\scantokens{a}')
        d.add(body + '%' + 'x' * (r.randint(199986, 199996) - len(body) - 1) + '\n')
    elif k == 2:
        # A long \csname on top of a full buffer.
        n = r.choice([1000, 150000, 199000, 199990, 200000, 200100, 400000])
        d.add('\\def\\a{%s}\\def\\b{\\a\\a\\a\\a}\\def\\c{\\b\\b\\b\\b}\n' % ('a' * 1000))
        d.add('\\edef\\t{\\csname %s\\endcsname}\n' % ('\\a ' * (n // 1000)))
        d.add('\\ifcsname %s\\endcsname\\fi\n' % ('\\a ' * (n // 1000)))
        d.add('\\scantokens\\expandafter{%s}\n' % ('\\a ' * (n // 1000)))
    elif k == 3:
        # Macro recursion to the end of the input stack, with arguments or not.
        d.add('\\count1=0 \\def\\r{\\advance\\count1 by 1 \\ifnum\\count1<%d \\r\\relax\\fi\\relax}\\r\n' % r.choice([100, 2000, 2490, 2500, 2510, 3000, 4990, 5000, 5010, 10000]))
        d.add('\\showthe\\count1\n')
    elif k == 4:
        # The parameter stack: nine arguments a level, or an empty one.
        n = r.choice([100, 1000, 2000, 2200, 2223, 2230, 2300, 2500])
        d.add('\\count1=0 \\def\\r#1#2#3#4#5#6#7#8#9{\\advance\\count1 by 1 \\ifnum\\count1<%d \\r#1#2#3#4#5#6#7#8{#9}\\relax\\fi#1\\relax}\\r 123456789\n' % n)
        d.add('\\showthe\\count1\n')
    elif k == 5:
        # Grouping levels, 255; and the semantic nest, and the save stack.
        n = r.choice([100, 254, 255, 256, 257, 300])
        d.add(r.pick('{' * n, '\\begingroup ' * n, '\\hbox{' * n, '\\vbox{' * n, '\\setbox0\\hbox{' * n, '$$\\vcenter{' * n) + '\n')
        d.add('\\showgroups\n')
        d.add('}' * n + '\n')
    elif k == 6:
        # Conditionals, \input levels, \scantokens levels.
        n = r.choice([10, 14, 15, 16, 17])
        d.add('\\count1=0 \\def\\r{\\advance\\count1 by 1 \\ifnum\\count1<%d \\scantokens{\\r}\\fi}\\r\n' % n)
        d.add('\\showthe\\count1 \\end\n')
    elif k == 7:
        # An argument of each size, undelimited, delimited, read from a list, and given on as a parameter, with a group across a boundary of a class.
        pass
    elif k == 8:
        # \expandafter's depth.
        n = r.choice([100, 9000, 9990, 10000, 10010, 20000])
        d.add('\\def\\z{}\\edef\\t{%s\\z}\n' % ('\\expandafter ' * r.choice([1, 2, 3])))
        d.add('\\def\\k#1{\\ifnum#1>0 \\expandafter\\expandafter\\expandafter\\k\\expandafter\\expandafter\\expandafter{\\expandafter\\number\\numexpr#1-1\\expandafter\\relax\\expandafter}\\fi}\\k{%d}\n' % min(n, 12000))
        d.add('%s\\relax\n' % ('\\expandafter ' * n))
    elif k == 9:
        # A \def whose body is long, in a definition or in \toks, with the end of the file before its end.
        n = l_sizes(r) % 300000
        d.add('\\def\\t{%s\n' % ('a' * n))
    elif k == 10:
        n = l_sizes(r) % 300000
        d.add('\\toks0={%s}\\edef\\t{\\the\\toks0 }\\edef\\u{\\the\\toks0 \\the\\toks0 }\\detokenize\\expandafter{\\the\\toks0 }\\message{\\the\\toks0 }\n' % ('a' * n))
        d.add('\\ifx\\t\\u\\fi \\show\\t\n')
    elif k == 11:
        # A word that fills the hyphenation buffer and a paragraph of long tokens: only for the checks on the way.
        n = r.choice([62, 63, 64, 65, 256, 1000])
        d.add('\\hyphenchar\\f=45 \\lccode`a=`a \\hsize=10pt ' + 'a' * n + '\\par\n')
    elif k == 12:
        # The save stack.
        n = r.choice([100, 1000, 5000])
        d.add('\\count1=0 \\def\\r{\\advance\\count1 by 1 \\begingroup\\count2=\\count1 \\ifnum\\count1<%d \\expandafter\\r\\fi}\\r\n' % n)
    elif k == 13:
        # The nesting of \if in an \edef.
        n = r.choice([100, 1000, 5000, 30000])
        d.add('\\edef\\t{%s}\n' % ('\\iftrue ' * n))
        d.add('\\fi ' * min(n, 3000) + '\n')
    elif k == 14:
        # A runaway: what the show of a long list in flight does.
        n = r.choice([10, 39, 40, 41, 69, 70, 71, 1000])
        d.add('\\m{%s\\par\n' % ('a' * n))
        d.add('\\d %s\\par\n' % ('a' * n))
        d.add('\\edef\\t{%s\n' % ('a' * n))
    elif k == 15:
        # Control sequences by the thousand: the hash table's growth.
        n = r.choice([100, 5000, 20000, 70000, 620000])
        d.add('\\count1=0 \\def\\r{\\expandafter\\def\\csname n\\number\\count1\\endcsname{}\\advance\\count1 by 1 \\ifnum\\count1<%d \\expandafter\\r\\fi}\\r\n' % n)
        d.add('\\ifcsname n1\\endcsname\\message{yes}\\fi \\show\\n1 \\expandafter\\show\\csname n%d\\endcsname\n' % (n - 1))
    if k == 7 or r.chance(0.3):
        # Tokens that differ, so that a piece lost or copied twice at the end of a block changes the sum in the log.
        n = l_sizes(r)
        arg = ''.join(LETTERS[k % 26] if k % 7 else '\\x ' for k in range(n))
        g = 'b{%s}c' % arg[:max(len(arg) - 4, 0)].rstrip('\\x ')
        macro = '\\e' if n > 2000 else '\\m'
        d.add('%s{%s}\n' % (macro, arg))
        d.add('\\ee %s.\n' % arg)
        d.add('\\def\\u#1{\\e{#1}\\ee#1.\\ee{#1}.}\\u{%s}\n' % arg)
        d.add('\\def\\u{\\e{%s}\\ee %s.\\ee %s.\\e%s}\\u\n' % (arg, arg, g, '{%s}' % g))
        d.add('\\def\\u#1#2{\\e{a#1b#2c#1}\\ee a#1b#2c#1.}\\u{%s}{%s}\\u{}{}\n' % (arg, g))
        d.add('\\def\\u{\\e{%s\\par}}\\u\n' % arg)
        d.add('\\def\\u{\\ee %s\\par}\\u\n' % arg)
        d.add('\\e{%s\n\n' % ('a' * 50))
    return Case({'f.tex': d.text()}, b'', r.pick('batchmode', 'nonstopmode'))


GENERATORS = {
    'macros': gen_macros, 'delims': gen_delims, 'outer': gen_outer, 'align': gen_align, 'files': gen_files, 'cond': gen_cond, 'expand': gen_expand,
    'scan': gen_scan, 'active': gen_active, 'trace': gen_trace, 'interact': gen_interact, 'limits': gen_limits, 'tables': gen_tables, 'stops': gen_stops, 'deep': gen_deep,
}


def small_options(r):
    """Capacities for --small: what is set is near the least that the program takes, and a little above, so that an overflow comes at any point of a document."""
    options = []
    for name, low, high in (('stack_size', 200, 240), ('param_size', 60, 90), ('buf_size', 500, 700), ('save_size', 600, 900), ('nest_size', 40, 60),
                            ('max_in_open', 6, 9), ('expand_depth', 10, 60)):
        if r.chance(0.35):
            options.append('-cnf-line=%s=%d' % (name, r.randint(low, high)))
    if r.chance(0.25):
        e = r.randint(45, 120)
        options += ['-cnf-line=error_line=%d' % e, '-cnf-line=half_error_line=%d' % r.randint(30, e - 15)]
    if r.chance(0.25):
        options.append('-cnf-line=max_print_line=%d' % r.randint(60, 200))
    return options


# ---------------------------------------------------------------------------------------------------------------------
# Running and judging.
# ---------------------------------------------------------------------------------------------------------------------
class Run:
    """What one program did with a document, masked."""

    def __init__(self, rc, log, out, err, pdf, others, seconds):
        self.rc, self.log, self.out, self.err, self.pdf, self.others, self.seconds = rc, log, out, err, pdf, others, seconds
        self.compressed = pdf is not None and (b'/FlateDecode' in pdf or b'/ObjStm' in pdf)


def write_case(case, work):
    shutil.rmtree(work, ignore_errors=True)
    os.makedirs(work)
    for name, text in case.files.items():
        with open(os.path.join(work, name), 'wb') as f:
            f.write(text.encode('latin-1') if isinstance(text, str) else text)


def pdf_masked(data):
    """Nothing in these is compressed, so bytes will do, but for the banner's length, which moves every offset."""
    data = re.sub(rb'\((This is pdfTeX, Version \S+ )\((?:TeX Live 2026/Arch Linux|rstex)\)', rb'(\1(VERSION)', data)
    data = re.sub(rb'\nxref\n.*?\nstartxref\n\d+\n', b'\nXREF\n', data, flags=re.S)
    return data


def limit_child():
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    resource.setrlimit(resource.RLIMIT_FSIZE, (OUTPUT_LIMIT, OUTPUT_LIMIT))


def build_format(program, directory, kind):
    """A format in directory/mini.fmt, for runs that find what a format holds: its strings, its names, and the sticky flag for \\outer."""
    os.makedirs(directory, exist_ok=True)
    with open(os.path.join(directory, 'mini.tex'), 'w') as f:
        f.write(PREAMBLE + ('\\outer\\def\\Ofmt{}' if kind == 'outer' else '\\def\\Ofmt{}') + '\\def\\Ffmt#1.{#1}\\font\\Gfmt=cmr10 \\dump\n')
    subprocess.run([os.path.basename(program), '-ini', '-etex', '-jobname=mini', '*mini.tex'], executable=program, cwd=directory, env=integration.environment(),
                   stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True)
    return os.path.join(directory, 'mini')


def execute(program, case, work, timeout, fmt=None):
    write_case(case, work)
    stdin = os.path.join(work, '.stdin')
    with open(stdin, 'wb') as f:
        f.write(case.stdin)
    env = integration.environment()
    start = time.monotonic()
    try:
        with open(stdin, 'rb') as i:
            start_of = ['-fmt=' + fmt] if fmt else ['-ini', '-etex']
            p = subprocess.run([os.path.basename(program)] + start_of + case.options + ['-interaction=' + case.interaction, '-jobname=f', 'f.tex' if fmt else '*f.tex'], executable=program, cwd=work, env=env,
                               stdin=i, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=timeout, preexec_fn=limit_child)
        rc, out, err = p.returncode, p.stdout, p.stderr
    except subprocess.TimeoutExpired:
        rc, out, err = 'timeout', b'', b''
    seconds = time.monotonic() - start
    pdfs = {}
    path = os.path.join(work, 'f.pdf')
    if os.path.exists(path):
        pdfs['f.pdf'] = integration.pdf_facts(path)
    log = open(os.path.join(work, 'f.log'), 'rb').read() if os.path.exists(os.path.join(work, 'f.log')) else b''
    masked = integration.mask_lines(log, pdfs, bool(fmt))
    pdf = pdf_masked(open(path, 'rb').read()) if os.path.exists(path) else None
    others = {}
    for name in sorted(os.listdir(work)):
        if name not in case.files and name not in ('.stdin', 'f.log', 'f.pdf') and not name.endswith('.fmt'):
            q = os.path.join(work, name)
            if os.path.isfile(q):
                others[name] = open(q, 'rb').read()
    return Run(rc, masked, integration.mask_lines(out, pdfs, bool(fmt)), integration.STDERR_DROP.sub(b'', err), pdf, others, seconds), log


def first_difference(a, b):
    """A sentence on the first thing in which two runs differ, or None."""
    if a.rc != b.rc:
        return 'exit status %s and %s' % (a.rc, b.rc)
    for what, x, y in (('log', a.log, b.log), ('terminal', a.out, b.out)):
        if x != y:
            k = next((i for i, (p, q) in enumerate(zip(x, y)) if p != q), min(len(x), len(y)))
            p = x[k][:150] if k < len(x) else '<end>'
            q = y[k][:150] if k < len(y) else '<end>'
            return '%s line %d\n  oracle: %s\n  engine: %s' % (what, k + 1, p, q)
    if a.err != b.err:
        return 'stderr\n  oracle: %r\n  engine: %r' % (a.err[:200], b.err[:200])
    # A document can turn compression on itself, and the deflate bytes are not the oracle's: PORTING section 18.  Such a one is compared by what the masks of its log leave.
    if a.pdf != b.pdf and not (a.compressed and b.compressed):
        if a.pdf is None or b.pdf is None:
            return 'one has a PDF and the other none'
        k = next((i for i, (p, q) in enumerate(zip(a.pdf, b.pdf)) if p != q), min(len(a.pdf), len(b.pdf)))
        return 'PDF at byte %d\n  oracle: %r\n  engine: %r' % (k, a.pdf[max(0, k - 40):k + 40], b.pdf[max(0, k - 40):k + 40])
    if a.others != b.others:
        return 'other files: %s and %s' % (sorted(a.others), sorted(b.others))
    return None


def capacity_passed_over(a, b):
    """The capacity a run reached that is another number on the two sides, if any."""
    found = []
    for run in (a, b):
        for line in run.log:
            m = CAPACITY.match(line)
            if m:
                found.append((m.group(1), m.group(2)))
    if not found:
        return None
    for name, _ in found:
        if name in OTHER_CAPACITY:
            return name
    if len(found) == 2 and found[0] == found[1]:
        return None
    return found[0][0]


# The formats of a run, when it has them, as (the oracle's, the engine's).
FORMATS = [None, None]


def judge(engine, case, work, timeout):
    """('same'|'differs'|'capacity:NAME'|'timeout', detail, oracle's run and raw log, engine's)."""
    a, araw = execute(ORACLE, case, work, timeout, FORMATS[0])
    b, braw = execute(engine, case, work, timeout, FORMATS[1])
    # Past the limit of a file: where each stops depends on how it buffers.
    if (a.rc == 'timeout' or a.rc == -signal.SIGXFSZ) and (b.rc == 'timeout' or b.rc == -signal.SIGXFSZ):
        return 'timeout', None, araw, braw
    capacity = capacity_passed_over(a, b)
    if capacity:
        return 'capacity:' + capacity, None, araw, braw
    detail = first_difference(a, b)
    # An assignment of what clamps to what stays there is no change here, and is one there.
    if detail and (any(past_the_clamp(n) for n in BEYOND.findall(detail.split('engine:')[0])) or ('reassigning' in detail and '2147483647' in detail)):
        return 'clamp', detail, araw, braw
    if detail:
        return 'differs', detail, araw, braw
    return ('overflow' if any(CAPACITY.match(line) for line in a.log) else 'same'), None, araw, braw


def kind_of(detail):
    """What a difference is in, so that a reduction does not wander off to another one."""
    return detail.split()[0] if detail else None


def reduce_case(engine, case, work, timeout, budget=1500):
    """Delta debugging on the lines of f.tex, then on its tokens, while the document differs in the same kind of thing.  Returns the case, and how many runs it took."""
    runs = [0]
    kind = kind_of(judge(engine, case, work, timeout)[1])

    def same_kind(c):
        verdict, detail = judge(engine, c, work, timeout)[:2]
        return verdict == 'differs' and kind_of(detail) == kind

    def differs(text):
        if runs[0] >= budget:
            return False
        runs[0] += 1
        return same_kind(Case(dict(case.files, **{'f.tex': text}), case.stdin, case.interaction, case.options))

    def shrink(items, joined):
        n = 2
        while len(items) >= 2 and runs[0] < budget:
            size = max(len(items) // n, 1)
            chunks = [items[k:k + size] for k in range(0, len(items), size)]
            for k in range(len(chunks)):
                rest = [x for j, chunk in enumerate(chunks) if j != k for x in chunk]
                if differs(joined(rest)):
                    items = rest
                    n = max(n - 1, 2)
                    break
            else:
                if size == 1:
                    break
                n = min(n * 2, len(items))
        return items

    text = case.files['f.tex']
    lines = shrink(text.split('\n'), '\n'.join)
    text = '\n'.join(lines)
    token = re.compile(r'\\[A-Za-z@]+ ?|\\.|\^\^.|.|\n', re.S)
    tokens = shrink(token.findall(text), ''.join)
    text = ''.join(tokens)
    # The other files, one at a time, if the case can do without them.
    files = dict(case.files, **{'f.tex': text})
    for name in list(files):
        if name != 'f.tex':
            trial = {k: v for k, v in files.items() if k != name}
            runs[0] += 1
            if same_kind(Case(trial, case.stdin, case.interaction, case.options)):
                files = trial
    if case.stdin:
        runs[0] += 1
        if same_kind(Case(files, b'', case.interaction, case.options)):
            return Case(files, b'', case.interaction, case.options), runs[0]
    return Case(files, case.stdin, case.interaction, case.options), runs[0]


def case_of(gen, seed):
    return GENERATORS[gen](Rand('%s:%d' % (gen, seed)))


def save_case(case, directory):
    os.makedirs(directory, exist_ok=True)
    for name, text in case.files.items():
        with open(os.path.join(directory, name), 'wb') as f:
            f.write(text.encode('latin-1') if isinstance(text, str) else text)
    with open(os.path.join(directory, 'stdin'), 'wb') as f:
        f.write(case.stdin)
    with open(os.path.join(directory, 'interaction'), 'w') as f:
        f.write(case.interaction)
    with open(os.path.join(directory, 'options'), 'w') as f:
        f.write(' '.join(case.options))


def load_case(directory):
    files = {}
    for name in sorted(os.listdir(directory)):
        if name.endswith('.tex') and not name.startswith('.'):
            files[name] = open(os.path.join(directory, name), 'rb').read().decode('latin-1')
    options = os.path.join(directory, 'options')
    return Case(files, open(os.path.join(directory, 'stdin'), 'rb').read(), open(os.path.join(directory, 'interaction')).read().strip(), open(options).read().split() if os.path.exists(options) else [])


def work_one(job):
    gen, seed, engine, base, timeout, reduce_it, small = job
    work = os.path.join(base, 'w%d' % os.getpid())
    case = case_of(gen, seed)
    if small:
        case.options += small_options(Rand('small:%s:%d' % (gen, seed)))
    verdict, detail, araw, braw = judge(engine, case, work, timeout)
    if verdict == 'differs' and judge(engine, case, work, timeout)[0] != 'differs':
        verdict = 'flake'
    if verdict == 'differs':
        kept = os.path.join(base, 'kept', '%s-%d' % (gen, seed))
        save_case(case, kept)
        with open(os.path.join(kept, 'oracle.log'), 'wb') as f:
            f.write(araw)
        with open(os.path.join(kept, 'engine.log'), 'wb') as f:
            f.write(braw)
        with open(os.path.join(kept, 'difference'), 'w') as f:
            f.write(detail + '\n')
        if reduce_it:
            small, _ = reduce_case(engine, case, work, timeout)
            save_case(small, os.path.join(kept, 'reduced'))
            verdict2, detail2, araw2, braw2 = judge(engine, small, work, timeout)
            if verdict2 == 'differs':
                with open(os.path.join(kept, 'reduced', 'difference'), 'w') as f:
                    f.write(detail2 + '\n')
                with open(os.path.join(kept, 'reduced', 'oracle.log'), 'wb') as f:
                    f.write(araw2)
                with open(os.path.join(kept, 'reduced', 'engine.log'), 'wb') as f:
                    f.write(braw2)
    shutil.rmtree(work, ignore_errors=True)
    return gen, seed, verdict, detail


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('engine', nargs='?')
    ap.add_argument('--gen', default=','.join(GENERATORS))
    ap.add_argument('--first', type=int, default=1)
    ap.add_argument('--count', type=int, default=100)
    ap.add_argument('-j', type=int, default=4)
    ap.add_argument('--run', default='default')
    ap.add_argument('--timeout', type=float, default=40)
    ap.add_argument('--no-reduce', action='store_true')
    ap.add_argument('--format', choices=['clean', 'outer'])
    ap.add_argument('--small', action='store_true')
    ap.add_argument('--again')
    ap.add_argument('--reduce')
    ap.add_argument('--list', action='store_true')
    args = ap.parse_args()
    if args.list:
        print(' '.join(GENERATORS))
        return 0
    engine = os.path.abspath(args.engine)
    base = os.path.join(ROOT, 'target', 'token-fuzz', args.run)
    os.makedirs(base, exist_ok=True)
    if args.format:
        FORMATS[0] = build_format(ORACLE, os.path.join(base, 'fmt-oracle'), args.format)
        FORMATS[1] = build_format(engine, os.path.join(base, 'fmt-engine'), args.format)
    if args.again or args.reduce:
        directory = args.again or args.reduce
        case = load_case(directory)
        work = os.path.join(base, 'w%d' % os.getpid())
        if args.reduce:
            small, runs = reduce_case(engine, case, work, args.timeout)
            save_case(small, os.path.join(directory, 'reduced'))
            print('%d runs; %s' % (runs, os.path.join(directory, 'reduced')))
            case = small
        verdict, detail, araw, braw = judge(engine, case, work, args.timeout)
        print(verdict)
        if detail:
            print(detail)
        with open(os.path.join(directory, 'oracle-again.log'), 'wb') as f:
            f.write(araw)
        with open(os.path.join(directory, 'engine-again.log'), 'wb') as f:
            f.write(braw)
        shutil.rmtree(work, ignore_errors=True)
        return 1 if verdict == 'differs' else 0
    gens = args.gen.split(',')
    jobs = [(g, s, engine, base, args.timeout, not args.no_reduce, args.small) for g in gens for s in range(args.first, args.first + args.count)]
    table = {g: {'documents': 0, 'differ': 0, 'capacity': 0, 'timeout': 0, 'clamp': 0, 'flake': 0, 'overflow': 0} for g in gens}
    capacities = {}
    # Fork, for the workers to have FORMATS: the default start method may not give it to them.
    with multiprocessing.get_context('fork').Pool(args.j) as pool:
        for gen, seed, verdict, detail in pool.imap_unordered(work_one, jobs, chunksize=1):
            row = table[gen]
            row['documents'] += 1
            if verdict == 'differs':
                row['differ'] += 1
                print('%s-%d: %s' % (gen, seed, detail), flush=True)
            elif verdict.startswith('capacity:'):
                row['capacity'] += 1
                capacities[verdict[9:]] = capacities.get(verdict[9:], 0) + 1
            elif verdict == 'timeout':
                row['timeout'] += 1
            elif verdict in ('clamp', 'flake', 'overflow'):
                row[verdict] += 1
    columns = ('documents', 'differ', 'capacity', 'timeout', 'clamp', 'flake', 'overflow')
    print('%-10s' % 'generator' + ''.join(' %9s' % c for c in columns))
    for g in gens:
        print('%-10s' % g + ''.join(' %9d' % table[g][c] for c in columns))
    total = {k: sum(table[g][k] for g in gens) for k in columns}
    print('%-10s' % 'all' + ''.join(' %9d' % total[c] for c in columns))
    print('overflow: compared to the end, a stack having ended at the same token on both sides; the others are counted as passed over, and in differ only what remains.')
    if capacities:
        print('passed over at: ' + ', '.join('%s %d' % kv for kv in sorted(capacities.items())))
    print('seeds %d to %d; kept under %s' % (args.first, args.first + args.count - 1, os.path.join(base, 'kept')))
    return 1 if total['differ'] else 0


if __name__ == '__main__':
    sys.exit(main())
