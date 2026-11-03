#!/usr/bin/env -S python3 -I
r"""Generates crates/engine/src/native_table.rs: what the native diagram path knows about the installed tikz-cd, PGF and LaTeX.  DESIGN section 10.

    cargo build --release -p rstex --features rstex-engine/commute-probe --target-dir target/native/probe
    python3 -I tests/cd/native/gen_table.py --engine target/native/probe/release/rstex

Four lists, all from runs of the engine with the path off:

  GUARD    Macros and keys that real tikz-cd runs or reads, with their hash.  tests/cd is compiled, and the probe of src/native_probe.rs says which macros were called and which
           names of keys were looked up.  Kept are those of PGF, TikZ and tikz-cd that exist at \begin{document} and are the same at its end and under every preamble in STABLE.
           What a document changes then shows as a hash that differs, and another version of PGF or tikz-cd shows in hundreds of them.  Also kept is what any key of /tikz or of
           tikz-cd's directory writes when \tikzset gives it a value outside a picture, by a run of each with a few values: what a diagram reads only by \ifx or \let is never called.
  ABSENT   Names of keys that real tikz-cd asks for and that do not exist in a plain preamble, like `every text node part`: a document that defines one changes the diagrams.
  REGISTERS Registers that a key writes outside a picture, and what they hold as installed: `shorten >` and its like.
  KEYS     Hashes of `.@cmd` of the keys of /tikz and of tikz-cd's directory under every preamble in STABLE, so with the libraries that they load: the library asks about a key it is given, and one with
           another hash is the document's.
  SAFE     Hashes of macros, name and body, that may stay in a cell without a look at what they mention: LaTeX's and those of the packages in SAFE_PREAMBLES, less what changes how text is read.
           A macro of the document, or of any other package, is walked instead, and a hash can't be had by a redefinition that says something else.

The output depends on the TeX tree, which is why the path is off on any other: the hashes fail, and a run gets slower and no worse.
"""
import argparse
import collections
import os
import re
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(os.path.dirname(HERE)))
OUT = os.path.join(REPO, 'crates', 'engine', 'src', 'native_table.rs')
CD = os.path.join(REPO, 'tests', 'cd')

# A document of the corpus that has one of these in its preamble is still recognised.  Whatever differs between them is not stock: scratch macros that the loading of a package leaves behind.
STABLE = [
    r'\documentclass{article}\usepackage{tikz-cd}',
    r'\documentclass{article}\usepackage{amsmath,amssymb}\usepackage{tikz-cd}',
    r'\documentclass[12pt]{amsart}\usepackage{hyperref}\usepackage{tikz-cd}',
    r'\documentclass{book}\usepackage{xcolor}\usepackage{tikz}\usetikzlibrary{cd,arrows,calc,positioning,shapes,fit,matrix,decorations.pathmorphing,decorations.markings}',
    r'\documentclass{scrartcl}\usepackage[T1]{fontenc}\usepackage{lmodern}\usepackage{microtype}\usepackage{amsmath,amsthm,mathtools}\usepackage{tikz-cd}',
    r'\documentclass{article}\usepackage[english]{babel}\usepackage{graphicx}\usepackage{tikz-cd}\usepackage{hyperref}\usepackage{cleveref}',
    r'\documentclass{article}\usepackage{tikz-cd}\usetikzlibrary{patterns,backgrounds,intersections,through,angles,quotes,babel,knots}',
    r'\documentclass{beamer}\usepackage{tikz-cd}',
]

# Without hyperref: its \ref and \label make links, whose rectangles come from the place of the text in TeX's eyes.
SAFE_PACKAGES = 'amsmath,amssymb,amsfonts,amsthm,mathtools,xcolor,graphicx,stmaryrd,mathrsfs,bm,eucal,enumerate,dsfont,bbm,mathabx,wasysym,relsize,nicefrac,xfrac,cancel,bbold,upgreek,centernot,accents,esint,yfonts,tensor,mleftright,scalerel,bigdelim,multirow,array,slashed,braket,physics,commath,xspace,ifthen,mathdots,tensor,bbding,marvosym,pifont,textcomp,ulem,soul,thmtools,thm-restate,MnSymbol'.split(',')
SAFE_PREAMBLES = [
    r'\documentclass{article}',
    r'\documentclass{report}',
    r'\documentclass{book}',
    r'\documentclass{amsart}',
    r'\documentclass{amsbook}',
    r'\documentclass{scrartcl}',
    r'\documentclass{scrbook}',
    r'\documentclass[12pt]{article}\usepackage[T1]{fontenc}\usepackage{lmodern}',
    r'\documentclass{article}\usepackage[T1]{fontenc}\usepackage{lmodern}\usepackage{microtype}',
]

# What changes how text is read, directly or through what it mentions.  Reading ahead would be wrong under it: requirement 3 of the path.
DANGEROUS = {'dospecials', '@sanitize', 'obeylines', 'obeyspaces', '@makeother', 'makeatletter', 'makeatother', 'input', '@input', '@@input', '@input@', 'InputIfFileExists', '@iinput', 'include', 'verbatim', 'verb',
             'ExplSyntaxOn', 'ExplSyntaxOff', 'usepackage', 'RequirePackage', 'documentclass'}
# These are looked at by name or walked, whoever mentions them.
# expl3's timestamp holds the clock: its hash would differ with every run of this.
NOT_SAFE_THEMSELVES = {'begin', 'end', 'end ', 'tikzcd', 'endtikzcd', 'par', 'arrow', 'ar', 'rar', 'lar', 'dar', 'uar', 'urar', 'ular', 'drar', 'dlar', 'c_sys_timestamp_str'}
# These mention such a thing only to load a font's description: nothing a cell reads is touched.
NOT_DANGEROUS = {'selectfont', 'pickup@font', 'define@newfont', 'try@load@fontshape', 'nfss@catcodes', 'DeclareFontShape', 'DeclareFontFamily', 'wrong@fontshape', 'maybe@load@fontshape', 'not@math@alphabet', 'use@mathgroup', 'mathversion', 'getanddefine@fonts', 'check@mathfonts', 'glb@settings'}
SCRATCH = re.compile(r'@tempa|@tempb|@tempc|@gtempa|reserved@|@x@|next$|@next')
AT_NAMES = ('@if', '@first', '@second', '@third', '@gobble', '@protected', '@testopt', '@xargdef', '@empty', '@tempa', '@nil', '@nameuse', '@namedef', 'protect', 'kernel@', 'single:', 'active:', 'PU-', 'PD1-', 'T1-', 'OT1-', 'TS1-')
SWITCHES = ['iftikzcd@mathmode', 'iftikz@handle@active@code', 'iftikz@handle@active@nodes', 'ifpgfrememberpicturepositiononpage', 'ifpgfmatrix']
# Always kept: the real \end has to come to \endtikzcd, this says which version is here, \color is to work with the stack as the path uses it, and appending to a style is told by what the style's body was.
ALWAYS = ['end', 'end ', '@checkend', 'pgfversion', 'set@color', 'reset@color'] + [f'pgfk@/tikz/commutative diagrams/every {k}/{h}' for k in ('arrow', 'label', 'cell', 'matrix', 'diagram') for h in ('.@cmd', '.@body')]
# Scratch macros that a real diagram or a preamble leaves in any state, whatever the test documents show.
# What a key leaves behind only for the moment, and the tips of `>`, which are told by what `pgf@ar@means@>` has.
TRACE = re.compile(r'pgfkeys|pgfmath|pgf@tmp|pgf@temp|tikz@temp|tikz@tmp|pgf@[xyz]|pgfutil|lastxy|tikz@lastx|tikz@lasty|tikz@expandcount|^pgf@ar@')
# And what is read only by what a diagram does not do: plots, grids, trees, labels and pins of nodes, edges, pics.  So a document's `\tikzset{level distance=1cm}` does not send its diagrams away.
UNREAD = re.compile(r'plot|mark|grid|parabola|grow|level|sibling|child|parent|pin|label|edge|@pic|pictext|quotes|blend|fade|shade@angle|xyz|radius|special|centered')
# All that a document's pictures commonly do.
PICTURES = r'''\begin{tikzpicture}[thick]
\node[draw,label=a,pin=b,alias=z] (a) at (0,0) {x}; \node[right=of a] (b) {y};
\draw[->,red] (a) to[bend left] node[above] {z} (b); \fill (a) circle (1pt); \path (a) edge (b);
\matrix[matrix of math nodes] {a & b \\ c & d \\};
\draw (0,0) .. controls (1,1) .. (2,0) -- cycle;
\foreach \i in {1,2} {\draw (\i,0) -- ++(0,1) arc (0:90:1);}
\draw[rounded corners,dashed,shorten >=1pt] (0,0) rectangle (1,1);
\node at (1,1) {$x$} child {node {y}};
\begin{scope}[xshift=1cm,opacity=.5] \draw (0,0) -- (1,1); \end{scope}
\coordinate (c) at (1,1); \draw (c) -- ($(c)+(1,0)$);
\end{tikzpicture}
\tikz \draw (0,0) -- (1,1);
\begin{tikzcd} A \arrow[r,"f"] \arrow[d,bend left] & B \arrow[loop right] \\ C \end{tikzcd}'''
NOT_STATE = re.compile(r'temp|tmp|unravelled|reserved|@next|@marshal|pgf@x|pgf@y|lastkey|currentkey|currentname|currentpath|currentvalue|pgfmathresult|pgfutil@in|@sh@|@decl@|@resource@|pgf@path@check@proper|pgfarrowstheparameters|pgfmathreciprocal|tikz@installcommands|tikz@uninstallcommands|tikz@lib@scope@check|tikz@options|tikz@node@begin@hook')


KEY_CMD = re.compile(r'pgfk@/tikz/.*/\.@cmd')
# What a key is tried with, for what it writes.
KEY_VALUES = ['', '=1pt', '=red', '=1', '=west', '={}']
# Keys that do something by themselves that is no setting.
KEY_SKIP = re.compile(r'^(\.|/)|/|^(insert path|pic|pics|edge|path|node|graphs|fill between|use|decorate|font|name|name prefix|name suffix)$')


def run(engine, first, cwd, env_extra=None):
    env = dict(os.environ, RSTEX_NO_SNAPSHOT='1', RSTEX_NO_COMMUTE='1', TEXINPUTS=CD + ':' + os.environ.get('TEXINPUTS', ''))
    p = subprocess.run([engine, '-progname=pdflatex', '-interaction=batchmode', '-jobname=x', first], cwd=cwd, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=600)
    return p.returncode


def installed(pkg, cls='sty'):
    return subprocess.run(['kpsewhich', f'{pkg}.{cls}'], capture_output=True).returncode == 0


def key_writes(engine, cwd, stock_keys):
    """For each key and value, what \\tikzset writes.  One document with a mark before each, and from the setting after it where one ends the run."""
    keys = sorted({re.fullmatch(r'pgfk@/tikz/(.*)/\.@cmd', n).group(1) for _, n in stock_keys})
    settings = []
    for key in keys:
        name, setter = (key[len('commutative diagrams/'):], 'tikzcdset') if key.startswith('commutative diagrams/') else (key, 'tikzset')
        if not KEY_SKIP.search(name):
            settings += [f'\\{setter}{{{name}{v}}}' for v in KEY_VALUES]
    tex, out = os.path.join(cwd, 'keys.tex'), os.path.join(cwd, 'keys.delta')
    rows, at, stopped = [], 0, 0
    while at < len(settings):
        with open(tex, 'w') as f:
            f.write(STABLE[0] + '\n\\begin{document}\n')
            for one in settings[at:]:
                f.write(f'\\message{{rstexprobe-mark}}\\begingroup{one}\\message{{rstexprobe-delta {out}}}\\endgroup\\par\n')
            f.write('\\end{document}\n')
        if os.path.exists(out):
            os.remove(out)
        run(engine, tex, cwd)
        done, row = 0, []
        if os.path.exists(out):
            with open(out, errors='surrogateescape') as f:
                for line in f:
                    line = line.rstrip('\n')
                    if line == '-':
                        rows.append(row)
                        row = []
                        done += 1
                    else:
                        row.append(tuple(line.split('\t', 2)))
        if at + done < len(settings):
            stopped += 1
        at += done + 1
    print(f'{len(settings)} settings of keys tried, {stopped} of them end the run')
    return rows


def picture_leaves(engine, cwd):
    """What real pictures and a real diagram leave changed for what comes after them."""
    tex, out = os.path.join(cwd, 'leaves.tex'), os.path.join(cwd, 'leaves.delta')
    with open(tex, 'w') as f:
        f.write(STABLE[0] + '\n\\usetikzlibrary{matrix,positioning,calc}\n\\begin{document}\n\\message{rstexprobe-mark}\n' + PICTURES + f'\n\\message{{rstexprobe-delta {out}}}\n\\end{{document}}\n')
    run(engine, tex, cwd)
    with open(out, errors='surrogateescape') as f:
        return {line.rstrip('\n').split('\t', 2)[2] for line in f if '\t' in line}


def safe_dump(engine, preamble, cwd, name):
    out = os.path.join(cwd, name)
    tex = os.path.join(cwd, name + '.tex')
    with open(tex, 'w') as f:
        f.write(preamble + '\n\\AddToHook{begindocument/end}{\\message{rstexprobe-safe ' + out + '}}\n\\begin{document}\n\\end{document}\n')
    run(engine, tex, cwd)
    rows = []
    if os.path.exists(out):
        for line in open(out, errors='surrogateescape'):
            parts = line.rstrip('\n').split('\t')
            if len(parts) >= 3:
                rows.append((parts[0], int(parts[1]), parts[2], parts[3].split(' ') if len(parts) > 3 else []))
    return rows


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--engine', required=True)
    ap.add_argument('--check', action='store_true', help='say whether the table is what this generates, write nothing')
    args = ap.parse_args()
    engine = os.path.abspath(args.engine)
    tmp = tempfile.mkdtemp(dir=os.path.join(REPO, 'target', 'native') if os.path.isdir(os.path.join(REPO, 'target', 'native')) else None)

    # What real tikz-cd runs.
    called, looked = set(), {}
    dynamic = set()
    docs = sorted(n for n in os.listdir(CD) if n.endswith('.tex') and not os.path.islink(os.path.join(CD, n)) and not n.startswith('23-'))
    for n in docs:
        out = os.path.join(tmp, n + '.seen')
        first = f'\\AddToHook{{begindocument/end}}{{\\message{{rstexprobe-begin}}}}\\AddToHook{{enddocument}}{{\\message{{rstexprobe-end {out}}}}}\\input{{{n}}}'
        run(engine, first, tmp)
        # These change keys for all that follows, on purpose: what changes there says nothing about the rest.
        changes = n[:2] in ('16', '20', '27', '17', '15', '25', '28', '29', '30', '31')
        for line in open(out, errors='surrogateescape'):
            kind, h0, h1, name = line.rstrip('\n').split('\t', 3)
            if kind == 'M':
                called.add(name)
                if h0 != h1 and not changes:
                    dynamic.add(name)
            else:
                looked.setdefault(name, []).append((h0, h1))
                if h0 != h1 and not changes:
                    dynamic.add(name)
    print(f'{len(docs)} documents: {len(called)} macros called, {len(looked)} names of keys, {len(dynamic)} changed by a diagram')
    print('  ' + ' '.join(sorted(dynamic))[:1200])

    # Their hash under each preamble.
    stock = None
    unstable = set()
    stock_keys = set()
    for k, pre in enumerate(STABLE):
        rows = {r[2]: r[0] for r in safe_dump(engine, pre, tmp, f'stable{k}')}
        if not rows:
            print(f'  preamble {k} gave nothing: {pre}', file=sys.stderr)
            continue
        stock_keys |= {(h, n) for n, h in rows.items() if KEY_CMD.fullmatch(n)}
        if stock is None:
            stock = rows
        else:
            unstable |= {n for n, h in stock.items() if rows.get(n, h) != h}
    print(f'{len(stock)} macros in the plain preamble, {len(unstable)} of them differ under another preamble')
    print('  ' + ' '.join(sorted(n for n in unstable if n in called or n in looked))[:1500])

    wanted = {n for n in called if n.startswith(('pgf', 'tikz'))} | {n for n in looked if n in stock}
    # The switches that say something about how a diagram comes out.  The others are for this moment or that.
    wanted |= {n for n in SWITCHES if n in stock}
    guard = sorted((n, stock[n]) for n in wanted if n in stock and n not in dynamic and n not in unstable and not NOT_STATE.search(n))
    guard += [(n, stock[n]) for n in ALWAYS if n in stock and n not in dict(guard)]
    # Only the keys that nearly every diagram tries: a name that comes from one diagram's options is a key of that diagram, which the library knows or declines.  Those of a row or a column
    # are the engine's to ask for, by number.
    absent = sorted(n for n, seen in looked.items() if n not in stock and n not in dynamic and len(seen) >= len(docs) // 2 and all(h1 == '0' * 16 for _, h1 in seen)
                    and re.fullmatch(r'pgfk@/(tikz|tikz/commutative diagrams|pgf)/[^/]+/\.@cmd', n) and not re.search(r'/(column|row|\d+-row)[ /]|/[rdlu\']+/\.@cmd', n))
    print(f'guard {len(guard)}, absent {len(absent)}, keys {len(stock_keys)}')
    if args.check:
        print('\n'.join(absent))

    # What may stay in a cell.
    safe = set()
    for k, pre in enumerate(SAFE_PREAMBLES):
        pkgs = [p for p in SAFE_PACKAGES if installed(p)]
        # mathabx and bbold and the like redefine a great deal, and each may stop another from loading: on their own as well.
        for j, plist in enumerate([[p for p in pkgs if p in ('amsmath', 'amssymb', 'amsfonts', 'amsthm', 'mathtools', 'xcolor', 'graphicx', 'stmaryrd', 'mathrsfs', 'bm', 'eucal', 'enumerate', 'dsfont', 'bbm', 'array', 'multirow', 'slashed', 'cancel', 'nicefrac', 'relsize', 'upgreek', 'tensor', 'accents')],
                                   [p for p in pkgs if p in ('xfrac', 'wasysym', 'centernot', 'mleftright', 'scalerel', 'bigdelim', 'braket', 'physics', 'esint', 'yfonts', 'commath', 'xspace', 'ifthen', 'mathdots', 'bbding', 'pifont', 'textcomp', 'ulem', 'soul')],
                                   [p for p in pkgs if p == 'mathabx'],
                                   [p for p in pkgs if p == 'bbold']]):
            if not plist:
                continue
            preamble = pre + ''.join(f'\\usepackage{{{p}}}' for p in plist)
            rows = safe_dump(engine, preamble, tmp, f'safe{k}-{j}')
            mentions = {}
            own = {}
            for h, flags, name, ment in rows:
                # What expl3 calls of expl3's is its own business, or every message that can be given would count, and scratch macros hold what they last held.
                mentions[name] = [m for m in ment if ':' not in m and not m.startswith(('__', 'l__', 'g__', 'c__')) and not SCRATCH.match(m)]
                own[name] = flags
            # Upward from what changes how text is read.
            bad = {n for n, f in own.items() if f and not SCRATCH.match(n)} | set(DANGEROUS)
            changed = True
            while changed:
                changed = False
                for n, ment in mentions.items():
                    if n not in bad and n not in NOT_DANGEROUS and any(m in bad for m in ment):
                        bad.add(n)
                        changed = True
            bad -= NOT_DANGEROUS
            for h, flags, name, ment in rows:
                # A cell, or a macro of the document, names what has no @ in it, and a few that \\newcommand and friends leave in its macros.  What the macros here call is for them to worry about, and a
                # document's macro that names anything else is walked.
                if name in bad or name in NOT_SAFE_THEMSELVES or name.startswith(('pgf', 'tikz', '__', 'g__', 'l__', 'c__')) or (':' in name and not name.startswith(('single:', 'active:'))):
                    continue
                if '@' in name and not name.startswith(AT_NAMES):
                    continue
                safe.add((h, name))
    print(f'safe {len(safe)}')

    def lit(n):
        return 'b"' + n.encode('utf-8', 'surrogateescape').decode('latin-1').replace('\\', '\\\\').replace('"', '\\"') + '"'

    # What \\tikzcdset{arrow style=...} makes of the guarded names.
    styles = []
    names = dict(guard)
    for style in ('Latin Modern', 'tikz'):
        rows = {r[2]: r[0] for r in safe_dump(engine, STABLE[0] + '\\tikzcdset{arrow style=' + style + '}', tmp, 'style' + style[0])}
        styles.append((style, sorted((n, h) for n, h in rows.items() if n.startswith('pgf@ar@means@tikzcd ') and names.get(n) not in (None, h))))
    print('styles: ' + ', '.join(f'{s}: {len(e)}' for s, e in styles))

    written = key_writes(engine, tmp, stock_keys)
    # Scratch is what nearly any key writes, and what a real picture leaves behind, which a later diagram in real PGF does not see as a setting.
    common = {n for n, c in collections.Counter(n for row in written for _, _, n in set(row)).items() if c > len(written) // 3} | picture_leaves(engine, tmp)
    names = {n: k for row in written for k, _, n in row if n not in common and not NOT_STATE.search(n) and not TRACE.search(n) and not UNREAD.search(n)}
    held = {n: int(v) for row in written for k, v, n in row if k == 'R'}
    more = sorted(n for n, k in names.items() if k in 'MS' and n in stock and n not in dict(guard) and n not in unstable and n not in dynamic)
    guard = sorted(guard + [(n, stock[n]) for n in more])
    registers = sorted(n for n, k in names.items() if k == 'R')
    print(f'{len(written)} settings of keys: {len(more)} macros and switches more to guard, {len(registers)} registers')
    print('  guard: ' + ' '.join(more)[:1500])
    print('  registers: ' + ' '.join(registers)[:1000])

    text = ['//! What the native diagram path knows without looking.  Generated by `tests/cd/native/gen_table.py`, which says what each list is.  Do not edit.', '#![forbid(unsafe_code)]', '']
    text.append('/// Macros and keys of tikz-cd, TikZ and PGF as installed: name and hash, `Tex::native_macro_hash`.')
    text.append(f'pub static GUARD: [(&[u8], u64); {len(guard)}] = [')
    for n, h in guard:
        text.append(f'    ({lit(n)}, 0x{h}),')
    text.append('];')
    text.append('')
    text.append('/// Names of keys that real tikz-cd asks for and a plain preamble does not have.')
    text.append(f'pub static ABSENT: [&[u8]; {len(absent)}] = [')
    for n in absent:
        text.append(f'    {lit(n)},')
    text.append('];')
    text.append('')
    text.append('/// Registers that a key of TikZ writes outside a picture, and what they hold as installed.')
    text.append(f'pub static REGISTERS: [(&[u8], i32); {len(registers)}] = [')
    for n in registers:
        text.append(f'    ({lit(n)}, {held[n]}),')
    text.append('];')
    text.append('')
    text.append("/// `arrow style=`: the names that are not as in `GUARD`, and what they have then.")
    text.append(f'pub static STYLES: [(&[u8], &[(&[u8], u64)]); {len(styles)}] = [')
    for style, entries in styles:
        text.append(f'    ({lit(style)}, &[')
        for n, h in entries:
            text.append(f'        ({lit(n)}, 0x{h}),')
        text.append('    ]),')
    text.append('];')
    text.append('')
    text.append('/// Keys of `/tikz` and tikz-cd\'s directory as TikZ, its libraries and tikz-cd define them, by hash of name and `.@cmd`, sorted by that hash.  The names are there to be read.')
    text.append(f'static KEYS: [(u64, &[u8]); {len(stock_keys)}] = [')
    for h, n in sorted(stock_keys):
        text.append(f'    (0x{h}, {lit(n)}),')
    text.append('];')
    text.append('')
    text.append('pub fn is_stock_key(h: u64) -> bool {')
    text.append('    KEYS.binary_search_by_key(&h, |e| e.0).is_ok()')
    text.append('}')
    text.append('')
    text.append('/// Macros that may stay in a cell whatever they mention, by hash of name and body, sorted by that hash.  The names are there to be read.')
    ordered = sorted(safe)
    text.append(f'static SAFE: [(u64, &[u8]); {len(ordered)}] = [')
    for h, n in ordered:
        text.append(f'    (0x{h}, {lit(n)}),')
    text.append('];')
    text.append('')
    text.append('pub fn is_safe(h: u64) -> bool {')
    text.append('    SAFE.binary_search_by_key(&h, |e| e.0).is_ok()')
    text.append('}')
    text.append('')
    new = '\n'.join(text)
    subprocess.run(['rm', '-rf', tmp])
    if args.check:
        same = open(OUT, encoding='latin-1', errors='replace').read() == new
        print(f'{OUT}: ' + ('as this generates it' if same else 'NOT what this generates'))
        return 0 if same else 1
    with open(OUT, 'w', encoding='latin-1', errors='replace') as f:
        f.write(new)
    print(f'wrote {OUT}: {sum(len(t) + 1 for t in text)} bytes')
    return 0


if __name__ == '__main__':
    sys.exit(main())
