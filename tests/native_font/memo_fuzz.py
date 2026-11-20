#!/usr/bin/env -S python3 -I
r"""memo_fuzz.py: the memo of expansions of crates/engine/src/case_memo.rs on documents made at random, against the same program with the memo off.

    python3 -I tests/native_font/memo_fuzz.py --engine WRAPPER [--docs 400] [--seed 0] [--work DIR] [-j 8]

WRAPPER is called as tests/corpus/run.py calls an engine, with -fmt=pdftex.  A document is plain TeX: eight helper names, each a macro, a protected
macro, a \let, a \chardef, \relax or undefined; four macros whose body is an \expanded of fragments that read those names in every way the recorder
has a rule for, running them, \noexpand, \ifx of two of them and of one against a marker, \ifdefined, \ifcsname, \csname, \string, \meaning,
\detokenize, \unexpanded, \ifcat, \if, a \count and a \toks; and 160 rounds, each perhaps a change to a helper, a register, \escapechar or the
\toks, and then two calls in a \message.  So a call comes again with what it read as it was, which is a replay, and with one thing changed, which
must not be.  Each document runs with RSTEX_CASE_MEMO_EAGER, with RSTEX_NO_CASE_MEMO and with RSTEX_CASE_MEMO_CHECK: the log, the terminal, the exit
status and the state line of RSTEX_PF_STATS have to be the same in all three, and no check may fail.  Its last line says how many calls were replayed:
a run with none has tested nothing.  A document that fails stays in the work directory, under its seed.
"""
import argparse
import hashlib
import os
import random
import re
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
H = 8
# The helpers' names: a control word has letters only.
N = 'abcdefgh'


def helper_def(r, i):
    # Mostly a macro of one of two bodies, so that two of them are often the same tokens and often not, with the same `eq_type`.
    j = r.randrange(H)
    a, b = N[i], N[j]
    if r.random() < 0.6:
        return r'\def\h%s{%s}' % (a, r.choice('xy'))
    return r.choice([
        r'\protected\def\h%s{x}' % a, r'\protected\def\h%s{y}' % a, r'\def\h%s{\h%s z}' % (a, b) if j > i else r'\def\h%s{m}' % a,
        r'\let\h%s\h%s' % (a, b), r'\chardef\h%s=65 ' % a, r'\let\h%s\relax' % a, r'\let\h%s\undefined' % a, r'\long\def\h%s{x}' % a,
        r'\def\h%s{m}' % a, r'\let\h%s=a' % a,
    ])


def fragment(r):
    i, j, k = N[r.randrange(H)], N[r.randrange(H)], 20 + r.randrange(4)
    return r.choice([
        r'\h%s ' % i, r'\noexpand\h%s ' % i, r'\ifx\h%s\h%s T\else F\fi ' % (i, j), r'\ifx\h%s\mk T\else F\fi ' % i,
        r'\ifdefined\h%s D\else U\fi ' % i, r'\ifcsname h%s\endcsname C\else N\fi ' % i, r'\string\h%s ' % i, r'\meaning\h%s ' % i,
        r'\detokenize{\h%s a}' % i, r'\number\count%d ' % k, r'\ifnum\count%d>5 G\else L\fi ' % k, r'\romannumeral\count%d ' % k,
        r'\the\toks20 ', r'\expandafter\noexpand\csname h%s\endcsname ' % i, '#1', '#2', r'\unexpanded{\h%s}' % i,
        r'\ifcat\noexpand\h%s\noexpand\h%s S\else R\fi ' % (i, j), r'\if\noexpand\h%s\noexpand\h%s E\else Q\fi ' % (i, j), 'k',
    ])


def doc(seed):
    r = random.Random(seed)
    L = [r'\catcode`\@=11 \tracingonline=0 \scrollmode', r'\def\mk{x}',
         r'\def\burn#1{\ifnum#1>0 \expandafter\burn\expandafter{\the\numexpr#1-1\expandafter}\fi}']
    for i in range(H):
        L.append(helper_def(r, i))
    for k in range(20, 24):
        L.append(r'\count%d=%d ' % (k, r.randrange(12)))
    L.append(r'\toks20={t\h%s}' % N[r.randrange(H)])
    S = 4
    for s in range(S):
        frags = ''.join(fragment(r) for _ in range(r.randrange(2, 7)))
        L.append(r'\def\site%s#1#2{\expanded{\burn{14}%s}}' % ('abcd'[s], frags))
    args = ['a', r'\hb ', r'\noexpand\hc ']
    for rnd in range(160):
        if r.random() < 0.18:
            c = r.randrange(6)
            if c <= 2:
                L.append(helper_def(r, r.randrange(H)))
            elif c == 3:
                L.append(r'\count%d=%d ' % (20 + r.randrange(4), r.randrange(12)))
            elif c == 4:
                L.append(r'\escapechar=%d ' % r.choice([92, 92, 47, -1]))
            else:
                L.append(r'\toks20={t\h%s}' % N[r.randrange(H)])
        for _ in range(2):
            s = 'abcd'[r.randrange(S)]
            L.append(r'\message{[%d:\site%s{%s}{%s}]}' % (rnd, s, r.choice(args), r.choice(args)))
    L.append(r'\end')
    return '\n'.join(L) + '\n'


ENV = {'PATH': os.environ['PATH'], 'HOME': os.environ['HOME'], 'SOURCE_DATE_EPOCH': '1700000000', 'FORCE_SOURCE_DATE': '1', 'TZ': 'UTC', 'LC_ALL': 'C', 'LANG': 'C',
       'RSTEX_NO_SNAPSHOT': '1', 'RSTEX_PF_STATS': '1'}
def run(d, extra):
    r = subprocess.run([ARGS.engine, '-fmt=pdftex', '-interaction=batchmode', 'f.tex'], cwd=d, env=dict(ENV, **extra), stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    err = r.stderr.decode('latin-1')
    log = open(os.path.join(d, 'f.log'), 'rb').read() if os.path.exists(os.path.join(d, 'f.log')) else b''
    st = re.search(r'state: mem [0-9a-f]+ (.*)', err)
    cm = re.search(r'cm: calls (\d+), replayed (\d+), recorded (\d+), stopped (\d+)', err)
    ck = re.search(r'checked (\d+), check failed (\d+)', err)
    return r.returncode, hashlib.sha256(log).hexdigest(), st.group(1) if st else None, cm.groups() if cm else None, ck.groups() if ck else None, r.stdout
def one(seed):
    d = os.path.join(ARGS.work, 's%05d' % seed)
    os.makedirs(d, exist_ok=True)
    open(os.path.join(d, 'f.tex'), 'w').write(doc(seed))
    on = run(d, {'RSTEX_CASE_MEMO_EAGER': '1'})
    off = run(d, {'RSTEX_NO_CASE_MEMO': '1'})
    chk = run(d, {'RSTEX_CASE_MEMO_CHECK': '1'})
    bad = []
    if (on[0], on[1], on[2], on[5]) != (off[0], off[1], off[2], off[5]):
        bad.append('on/off differ')
    if (chk[0], chk[1], chk[2], chk[5]) != (off[0], off[1], off[2], off[5]):
        bad.append('check/off differ')
    if chk[4] and chk[4][1] != '0':
        bad.append('check failed %s' % chk[4][1])
    return seed, bad, on[3], chk[4]


def main():
    global ARGS
    ap = argparse.ArgumentParser()
    ap.add_argument('--engine', required=True)
    ap.add_argument('--docs', type=int, default=400)
    ap.add_argument('--seed', type=int, default=0)
    ap.add_argument('--work')
    ap.add_argument('-j', '--jobs', type=int, default=8)
    ARGS = ap.parse_args()
    ARGS.engine = os.path.abspath(ARGS.engine)
    ARGS.work = os.path.abspath(ARGS.work or os.path.join(REPO, 'target', 'memo-fuzz'))
    tot, nbad = [0, 0, 0, 0], 0
    with ThreadPoolExecutor(ARGS.jobs) as ex:
        for seed, bad, cm, ck in ex.map(one, range(ARGS.seed, ARGS.seed + ARGS.docs)):
            if cm:
                for i in range(4):
                    tot[i] += int(cm[i])
            if bad:
                nbad += 1
                print('FAIL seed %d: %s' % (seed, '; '.join(bad)), flush=True)
    print('memo_fuzz: %d documents, %d failed; %d calls, %d replayed, %d recorded, %d stopped' % (ARGS.docs, nbad, *tot))
    sys.exit(1 if nbad or not tot[1] else 0)


ARGS = None
if __name__ == '__main__':
    main()
