#!/usr/bin/env -S python3 -I
"""capture_fuzz.py: random macro programs on the oracle and on an engine, logs compared.

    python3 -I tests/capture_fuzz.py ENGINE [FIRST_SEED [COUNT]]

Made for crates/engine/src/capture.rs: macros with random parameter texts, partial matches, parameters in bodies and groups, called with random
arguments from token lists, in boxes, in cells, in an \\edef and in skipped text, with \\par, tabs, \\cr, conditionals and an \\outer that comes late.  Most of
what it writes is an error, which is the point.  A document whose log says "TeX capacity exceeded" on either side is passed over, as arguments
double and capacities differ.  It writes only under target/capture-fuzz, and keeps a document that differs there.  Exit status: 1 if any differs.
"""
import os
import random
import re
import shutil
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, 'target', 'capture-fuzz')
ORACLE = '/usr/bin/pdftex'
ATOMS = ['a', 'b', 'c', 'a', 'b', ' ', '.', ',', '\\x ', '\\y ', '\\relax ', '~', '\\amp ', '\\hash ', '\\lb ', '\\rb ', '\\space ', '\\undefinedcs ', '\\pr ']
RARE = ['\\par ', '&', '\\cr ', '\\span ', '\\crr ', '\\O ', '\\noexpand ', '\\noexpand\\x ', '\\noexpand~', '\\iftrue ', '\\iffalse ', '\\fi ', '\\else ', '\\or ',
        '\\yfi ', '\\expandafter ', '\\the\\toks0 ', '\\string ', '\\unexpanded{a\\x}', '\\detokenize{a\\x}', '\\number 12 ', '\\csname x\\endcsname ', '\\endgroup ', '\\begingroup ']
DELIMITERS = ['.', ',', 'a', 'ab', 'aab', 'aba', '\\x ', '\\x\\y ', '\\relax ', ' ', '\\par ', 'a\\x ', '\\x a', '\\x\\x\\y ']
PREAMBLE = r'''\catcode`\{=1 \catcode`\}=2 \catcode`\#=6 \catcode`\^=7 \catcode`\&=4 \catcode`\~=13 \catcode`\@=11
\tracingmacros=1 \tracingonline=0 \errorcontextlines=20 \tracingstats=1
\def\x{}\def\y{}\def~{}\def\space{ }\let\amp=&\let\hash=#\let\lb={\let\rb=}\let\crr=\cr\let\yfi=\fi\protected\def\pr{}
\def\O{plain}\toks0={a\x}
\def\Z{\noindent\par}
'''
# What depends on the pool and on capacities: PORTING section 15, "Known divergences".
SKIP = re.compile(r' \d+ strings? out of|^ \d+ string characters|^ \d+ words of memory|^ \d+ multiletter|^ \d+ words of font|^ \d+ hyphenation')


def text(r, depth, params, macros, rare):
    out = []
    for _ in range(r.randint(0, 6)):
        k = r.random()
        if k < 0.12 and depth > 0:
            out.append('{' + text(r, depth - 1, params, macros, rare) + '}')
        elif k < 0.32 and params:
            out.append('#%d' % r.randint(1, params))
        elif k < 0.55 and macros:
            out.append(call(r, depth, params, macros, rare))
        elif k < 0.55 + rare:
            out.append(r.choice(RARE))
        else:
            out.append(r.choice(ATOMS))
    return ''.join(out)


def call(r, depth, params, macros, rare):
    name, spec = r.choice(macros)
    out = [name + ' ']
    for delim in spec:
        arg = text(r, max(depth - 1, 0), params, macros, rare) if depth > 0 else r.choice(ATOMS)
        if delim is None:
            out.append('{' + arg + '}' if r.random() < 0.6 else r.choice(['a', '\\x ', '\\relax ', ' a', '{}', '#1' if params else 'b']))
        else:
            out.append(arg + delim if r.random() < 0.85 else arg)
    return ''.join(out)


def define(r, i, macros, rare):
    letters = 'abcdefghijklmnopqrstuvwxyz'
    name = '\\m' + letters[i // 26] + letters[i % 26]
    n = r.randint(0, 4)
    spec, head = [], ''
    for k in range(n):
        head += '#%d' % (k + 1)
        delim = r.choice(DELIMITERS) if r.random() < 0.5 else None
        head += delim or ''
        spec.append(delim)
    tail = '#' if r.random() < 0.05 else ''
    prefix = r.choice(['', '', '', '\\long', '\\long', '\\protected'])
    kind = r.choice(['\\def', '\\def', '\\def', '\\def', '\\edef', '\\gdef'])
    body = text(r, 2, n, macros, rare)
    macros.append((name, spec))
    return '%s%s%s %s%s{%s}\n' % (prefix, kind, name, head, tail, body)


def document(seed):
    r = random.Random(seed)
    rare = r.choice([0.0, 0.02, 0.06, 0.15])
    outer = r.random() < 0.3
    macros, lines = [], [PREAMBLE]
    for i in range(r.randint(10, 60)):
        lines.append(define(r, i, macros, rare))
        for _ in range(r.randint(0, 2)):
            c = call(r, 2, 0, macros, rare) if macros else ''
            if outer and r.random() < 0.1:
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
    return ''.join(lines)


def run(engine, tex):
    """In one directory for both, so that a wrapped path wraps alike."""
    work = os.path.join(OUT, 'w')
    shutil.rmtree(work, ignore_errors=True)
    os.makedirs(work)
    with open(os.path.join(work, 'f.tex'), 'w') as f:
        f.write(tex)
    env = dict(os.environ, SOURCE_DATE_EPOCH='1700000000', FORCE_SOURCE_DATE='1')
    try:
        rc = subprocess.run([engine, '-ini', '-etex', '-interaction=batchmode', '-jobname=f', '*f.tex'], cwd=work, env=env, stdin=subprocess.DEVNULL,
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=60).returncode
    except subprocess.TimeoutExpired:
        rc = 'timeout'
    path = os.path.join(work, 'f.log')
    log = open(path, 'rb').read().decode('latin1') if os.path.exists(path) else ''
    return rc, log


def main():
    engine = os.path.abspath(sys.argv[1])
    first = int(sys.argv[2]) if len(sys.argv) > 2 else 1
    count = int(sys.argv[3]) if len(sys.argv) > 3 else 100
    bad = passed_over = 0
    for seed in range(first, first + count):
        tex = document(seed)
        a, b = run(ORACLE, tex), run(engine, tex)
        if 'TeX capacity exceeded' in a[1] or 'TeX capacity exceeded' in b[1] or a[0] == b[0] == 'timeout':
            passed_over += 1
            continue
        la = [x for x in a[1].split('\n')[1:] if not SKIP.match(x)]
        lb = [x for x in b[1].split('\n')[1:] if not SKIP.match(x)]
        if a[0] != b[0] or la != lb:
            bad += 1
            k = next((i for i, (x, y) in enumerate(zip(la, lb)) if x != y), min(len(la), len(lb)))
            print('seed %d: exit %s and %s, log line %d' % (seed, a[0], b[0], k))
            print('  oracle:', la[k][:150] if k < len(la) else '<end>')
            print('  engine:', lb[k][:150] if k < len(lb) else '<end>')
            with open(os.path.join(OUT, 'bad-%d.tex' % seed), 'w') as f:
                f.write(tex)
    print('%d documents, %d differ, %d passed over' % (count, bad, passed_over))
    sys.exit(1 if bad else 0)


if __name__ == '__main__':
    main()
