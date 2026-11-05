#!/usr/bin/env -S python3 -I
r"""Generates crates/engine/src/native_table.rs: what the native diagram path knows about the installed tikz-cd, PGF and LaTeX, for each version it has been verified on.  DESIGN section 10.

    cargo build --release -p rstex --features rstex-engine/commute-probe --target-dir target/native/probe
    python3 -I tests/cd/native/gen_table.py --engine target/native/probe/release/rstex [--name ID] [--check]
    python3 -I tests/cd/native/gen_table.py --drop ID
    python3 -I tests/cd/native/gen_table.py --list

A profile is what one tree gives, as a whole: the path takes a diagram only if the macros of one profile are all there, and no combination of two was ever verified.  A run here makes the profile of the tree that kpsewhich
finds on PATH and puts it in the table beside the others, in place of one of the same name.  The name comes from the versions it finds, `pgf3.1.12-latex2026-11-01-pl0`, unless --name says.  What is in the table has
passed tests/cd/native/new_profile.py, which is what adds one and which runs this: a profile that has not passed that does not belong in a release.  --check says whether the table holds what this tree gives, and writes nothing.

Five lists, all from runs of the engine with the path off:

  GUARD    Macros and keys that real tikz-cd runs or reads, with their hash.  tests/cd is compiled, and the probe of src/native_probe.rs says which macros were called and which
           names of keys were looked up.  Kept are those of PGF, TikZ and tikz-cd that exist at \begin{document} and are the same at its end and under every preamble in STABLE.
           What a document changes then shows as a hash that differs, and another version of PGF or tikz-cd shows in hundreds of them.  Also kept is what any key of /tikz or of
           tikz-cd's directory writes when \tikzset gives it a value outside a picture, by a run of each with a few values: what a diagram reads only by \ifx or \let is never called.
           And what the kept macros say of LaTeX's and of the primitives: real PGF runs those too, more often than the path.
  ABSENT   Names of keys that real tikz-cd asks for and that do not exist in a plain preamble, like `every text node part`: a document that defines one changes the diagrams.
  REGISTERS Registers that a key writes outside a picture, and what they hold as installed: `shorten >` and its like.
  KEYS     Hashes of `.@cmd` of the keys of /tikz and of tikz-cd's directory under every preamble in STABLE, so with the libraries that they load: the library asks about a key it is given, and one with
           another hash is the document's.
  SAFE     Hashes of macros, name and body, that may stay in a cell without a look at what they mention: LaTeX's and those of the packages in SAFE_PREAMBLES, less what changes how text is read.
           A macro of the document, or of any other package, is walked instead, and a hash can't be had by a redefinition that says something else.
  KNOWN    Hashes of all macros of those preambles but PGF's and scratch: what a macro of SAFE mentions, and all that goes on to mention, must be one of these or be judged as a macro of the document is.
           A macro of expl3's is left alone only if it is one of these.

The output depends on the TeX tree, which is why the path is off on any other: the hashes fail, and a run gets slower and no worse.  `rstex --check-tree` says so to anyone who asks.
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
    # What PGF writes as it reads xcolor's model.
    r'\documentclass{article}\usepackage[cmyk]{xcolor}\usepackage{tikz-cd}',
    r'\documentclass{article}\usepackage[rgb]{xcolor}\usepackage{tikz-cd}',
    r'\documentclass{article}\usepackage[gray]{xcolor}\usepackage{tikz-cd}',
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
NOT_SAFE_THEMSELVES = {'begin', 'end', 'end ', 'tikzcd', 'endtikzcd', 'par', 'arrow', 'ar', 'rar', 'lar', 'dar', 'uar', 'urar', 'ular', 'drar', 'dlar', 'c_sys_timestamp_str',
                       # Vertical material and the structure of a page: errors in a formula, which real PGF and the path report in different words.
                       'newpage', 'clearpage', 'cleardoublepage', 'pagebreak', 'nopagebreak', 'enlargethispage', 'vspace', 'addvspace', 'smallskip', 'medskip', 'bigskip', 'vfill', 'endgraf', 'hrule', 'hline', 'cline', 'item', 'part', 'chapter', 'section',
                       'subsection', 'subsubsection', 'paragraph', 'subparagraph', 'maketitle', 'tableofcontents', 'listoffigures', 'listoftables', 'appendix', 'caption', 'thispagestyle', 'pagestyle', 'marginpar', 'footnotetext', 'newline', 'linebreak',
                       'centering', 'raggedright', 'raggedleft', 'noindent', 'indent', 'bibliography', 'printindex', 'boldmath', 'unboldmath', 'mathversion', 'multispan', 'multicolumn', 'omit', 'span', 'cr', 'crcr', 'noalign', 'hidewidth',
                       'tracingall', 'tracingnone', 'loggingall', 'showoutput', 'showlists', 'showhyphens', 'eqno', 'leqno', 'center', 'flushleft', 'flushright', 'trivlist', 'list', 'quote', 'quotation', 'verse', 'itemize', 'enumerate', 'description', 'figure', 'figure*', 'table', 'table*', 'equation', 'equation*', 'eqnarray', 'eqnarray*',
                       'align', 'align*', 'flalign', 'flalign*', 'alignat', 'alignat*', 'gather', 'gather*', 'multline', 'multline*', 'displaymath', 'abstract', 'titlepage', 'thebibliography', 'theindex', 'proof'}
# These mention such a thing only to load a font's description: nothing a cell reads is touched.
NOT_DANGEROUS = {'selectfont', 'pickup@font', 'define@newfont', 'try@load@fontshape', 'nfss@catcodes', 'DeclareFontShape', 'DeclareFontFamily', 'wrong@fontshape', 'maybe@load@fontshape', 'not@math@alphabet', 'use@mathgroup', 'mathversion', 'getanddefine@fonts', 'check@mathfonts', 'glb@settings'}
SCRATCH = re.compile(r'@tempa|@tempb|@tempc|@gtempa|reserved@|@x@|next$|@next')
# What holds something of the run that dumps it, for KNOWN: the date, the hook that says where to dump, the last argument specification.
UNSTEADY = {'c_sys_timestamp_str', '__hook_toplevel begindocument/end', 'l__cmd_args_tl'}
AT_NAMES = ('@if', '@first', '@second', '@third', '@gobble', '@protected', '@testopt', '@xargdef', '@empty', '@tempa', '@nil', '@nameuse', '@namedef', 'protect', 'kernel@', 'single:', 'active:', 'PU-', 'PD1-', 'T1-', 'OT1-', 'TS1-')
SWITCHES = ['iftikzcd@mathmode', 'iftikz@handle@active@code', 'iftikz@handle@active@nodes', 'ifpgfrememberpicturepositiononpage', 'ifpgfmatrix']
# Always kept: the real \end has to come to \endtikzcd, this says which version is here, \color is to work with the stack as the path uses it, and appending to a style is told by what the style's body was.
ALWAYS = ['end', 'end ', '@checkend', 'endtikzcd', 'pgfversion', 'set@color', 'reset@color'] + [f'pgfk@/tikz/commutative diagrams/every {k}/{h}' for k in ('arrow', 'label', 'cell', 'matrix', 'diagram') for h in ('.@cmd', '.@body')]
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
NOT_STATE = re.compile(r'^@currenvir$|^@pdfcolorstack$|^par$|^@ignorefalse$|temp|tmp|unravelled|reserved|@next|@marshal|pgf@x|pgf@y|lastkey|currentkey|currentname|currentpath|currentvalue|pgfmathresult|pgfutil@in|@sh@|@decl@|@resource@|pgf@path@check@proper|pgfarrowstheparameters|pgfmathreciprocal|tikz@installcommands|tikz@uninstallcommands|tikz@lib@scope@check|tikz@options|tikz@node@begin@hook')


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
    """What real pictures and a real diagram leave changed for what comes after them.  And all the diagrams of tests/cd, but for those that set things on purpose."""
    tex, out = os.path.join(cwd, 'leaves.tex'), os.path.join(cwd, 'leaves.delta')
    with open(tex, 'w') as f:
        f.write(STABLE[0] + '\n\\usetikzlibrary{matrix,positioning,calc}\n\\begin{document}\n\\message{rstexprobe-mark}\n' + PICTURES + f'\n\\message{{rstexprobe-delta {out}}}\n\\end{{document}}\n')
    run(engine, tex, cwd)
    with open(out, errors='surrogateescape') as f:
        leaves = {line.rstrip('\n').split('\t', 2)[2] for line in f if '\t' in line}
    for n in sorted(n for n in os.listdir(CD) if n.endswith('.tex') and not os.path.islink(os.path.join(CD, n)) and n[:2] not in ('16', '20', '27', '17', '15', '25', '28', '29', '30', '31', '23')):
        out = os.path.join(cwd, n + '.delta')
        run(engine, f'\\AddToHook{{begindocument/end}}{{\\message{{rstexprobe-mark}}}}\\AddToHook{{enddocument}}{{\\message{{rstexprobe-delta {out}}}}}\\input{{{n}}}', cwd)
        if os.path.exists(out):
            with open(out, errors='surrogateescape') as f:
                leaves |= {line.rstrip('\n').split('\t', 2)[2] for line in f if '\t' in line}
    return leaves


# What safe_dump found of the primitives: per preamble, name and hash.  And what each macro says that is no macro.
PRIMITIVES = {}
OTHERS = {}


# What pictures in general leave behind, of what PICTURES above does not happen to do.
SCRATCH_OF_PICTURES = re.compile(r'next|@decl@|unravelled|untranslated|theparameters|@options$|let@command|current bounding box')
# Libraries that real tikz-cd does not depend on define these again, and the corpus has them: nothing it draws differs.
NOT_GUARDED = {'pgf@arrow@tip@sequence', 'pgf@expanded@path', 'tikz@lib@graph@parser', 'tikz@svg@path', 'tikz@lib@dec@decorate@path', 'tikz@lib@decoration', 'tikz@parse@calculator', 'tikz@finish', 'pgfgetpath', 'pgfsetpath', 'everycr', 'tikz@installcommands', 'tikz@uninstallcommands', 'pgf@savedpar', 'tikz@node@begin@hook', 'tikz@node@reset@hook', 'tikz@lib@scope@check'}
# What tells one LaTeX from another, where no PGF is loaded.
LATEX_IDENTITY = ['fmtversion', 'patch@level']
# What a real diagram and the keys read but no key writes.
ALWAYS_REGISTERS = {'pgfarrowsep': 0}
# Told to the library, or what any key or call leaves for the moment.
NOT_ANY = {'pgf@layerlist', 'pgfkeys@code', 'pgfmathcounter', 'pgf@arrows@options', 'pgfkeyscurrentpath', 'pgfkeyscurrentname', 'pgfkeyscurrentkey', 'pgfkeyscurrentvalue', 'pgfkeyscurrentkeyRAW', 'pgfkeys@temp', 'pgfkeysvalueof'}


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
                if parts[1] == '2':
                    PRIMITIVES.setdefault(name, {})[parts[2]] = parts[0]
                    continue
                rows.append((parts[0], int(parts[1]), parts[2], parts[3].split(' ') if len(parts) > 3 else []))
                if len(parts) > 4:
                    OTHERS.setdefault(parts[2], set()).update(x for x in parts[4].split(' ') if x)
    return rows


# ------------------------------------------------------------------------------------------------ what a tree is

def tree_identity(engine, cwd):
    """The versions, what tells this LaTeX from another, and how this PGF writes a length: both ways the library knows are held against the tree's own `\\pgf@sys@bp`, or there is no profile."""
    values = [0, 1, -1, 2, -2, 26214, 32768, 65536, -65536, 12345, -12345, 100000, 805306, 26122, 39321, 170393, -170393, 1073741823, -1073741823]
    n = 7
    while n < 1000000000:
        values += [n, -n]
        n = 3 * n + 11
    one = ''.join(f'\\gdef\\pgfsysprotocol@currentprotocol{{}}\\pgf@sys@bp{{{v}sp}}\\typeout{{[[bp {v} \\pgfsysprotocol@currentprotocol]]}}\n' for v in values)
    tex = os.path.join(cwd, 'tree.tex')
    with open(tex, 'w') as f:
        f.write('\\documentclass{article}\\usepackage{tikz-cd}\n\\makeatletter\n\\begin{document}\n\\typeout{[[pgf \\pgfversion\\space\\pgfversiondate]]}\n\\typeout{[[latex \\fmtversion\\space patch level \\patch@level]]}\n' + one + '\\end{document}\n')
    run(engine, tex, cwd)
    with open(os.path.join(cwd, 'x.log'), encoding='latin-1') as f:
        log = re.sub(r'\n(?=[^\n]{0,78}\]\])', '', f.read())
    pgf = re.search(r'\[\[pgf ([^\]]*)\]\]', log)
    latex = re.search(r'\[\[latex ([^\]]*)\]\]', log)
    if not pgf or not latex:
        sys.exit('no versions in the log of a run with tikz-cd: is it installed?')
    written = {int(a): b for a, b in re.findall(r'\[\[bp (-?\d+) (-?[\d.]+) ?\]\]', log)}
    if len(written) != len(set(values)):
        sys.exit(f'{len(written)} lengths written of {len(set(values))}: this PGF has no \\pgf@sys@bp that the path knows of')

    def print_scaled(x):
        out = ('-' if x < 0 else '') + str(abs(x) >> 16) + '.'
        x, delta = 10 * (abs(x) % 65536) + 5, 10
        while True:
            if delta > 65536:
                x += 32768 - 50000
            out += str(x // 65536)
            x, delta = 10 * (x % 65536), delta * 10
            if x <= delta:
                return out

    def factor(x):
        return (1 if x > 0 else -1) * (abs(x) * 65292 // 65536)

    def exact(x):
        return (1 if x > 0 else -1) * (((2 * abs(x) + 1) * 800 + 803) // 1606)
    rules = [name for name, f in (('Factor', factor), ('Exact', exact)) if all(print_scaled(f(v)) == w for v, w in written.items())]
    if len(rules) != 1:
        sys.exit(f'this PGF writes lengths in a way the library does not know: commute::BpRule has to learn it.  Of {len(written)} lengths, by the rules it knows, {rules}')
    rows = safe_dump(engine, STABLE[0], cwd, 'identity')
    hashes = {r[2]: r[0] for r in rows}
    return pgf.group(1), latex.group(1), [(n, hashes[n]) for n in LATEX_IDENTITY if n in hashes], rules[0]


# ------------------------------------------------------------------------------------------------ the table

def lit_text(n):
    def one(c):
        return '\\\\' if c == 92 else '\\"' if c == 34 else chr(c) if 32 <= c < 127 else f'\\x{c:02x}'
    return ''.join(one(c) for c in n.encode('utf-8', 'surrogateescape'))


def lit(n):
    return 'b"' + lit_text(n) + '"'


def unlit(t):
    out, i = bytearray(), 0
    while i < len(t):
        if t[i] == '\\':
            if t[i + 1] == 'x':
                out.append(int(t[i + 2:i + 4], 16))
                i += 4
            else:
                out.append(ord(t[i + 1]))
                i += 2
        else:
            out.append(ord(t[i]))
            i += 1
    return out.decode('utf-8', 'surrogateescape')


HEAD = """//! What the native diagram path knows without looking, for each set of macros it has been verified on.  Generated by `tests/cd/native/gen_table.py`, which says what each list is, and added to by `tests/cd/native/new_profile.py`.  Do not edit.
#![forbid(unsafe_code)]

use commute::BpRule;

/// One tree's macros, as a whole: a diagram takes the path only if what it reads is as one of these has it.
pub struct Profile {
    pub id: &'static str,
    pub pgf: &'static str,
    pub latex: &'static str,
    /// How this PGF writes a length.
    pub bp: BpRule,
    /// Macros and keys of tikz-cd, TikZ and PGF as installed: name and hash, `Tex::native_macro_hash`.
    pub guard: &'static [(&'static [u8], u64)],
    /// Macros of TikZ and PGF that real tikz-cd calls and that differ from one preamble to another, as libraries define them: they must be as one of those has them, by `KNOWN`.
    pub guard_any: &'static [&'static [u8]],
    /// Names of keys that real tikz-cd asks for and a plain preamble does not have.
    pub absent: &'static [&'static [u8]],
    /// Registers that a key of TikZ writes outside a picture, and what they hold as installed.
    pub registers: &'static [(&'static [u8], i32)],
    /// `arrow style=`: the names that are not as in `guard`, and what they have then.
    pub styles: &'static [(&'static [u8], &'static [(&'static [u8], u64)])],
    /// What tells this LaTeX from another, where no PGF is loaded.
    pub latex_id: &'static [(&'static [u8], u64)],
}

"""


def row_list(name, kind, rows, count=None):
    return [f'static {name}: [{kind}; {len(rows) if count is None else count}] = ['] + rows + ['];', '']


def merged(profiles, what, name, note):
    """A list by hash over all profiles: the hashes with what each is, as a comment to be read, and which profiles have it, by bit."""
    bits, names = {}, {}
    for k, q in enumerate(profiles):
        for h, n in q[what].items():
            bits[h] = bits.get(h, 0) | 1 << k
            names[h] = n
    order = sorted(bits)
    text = [f'/// {note}', f'static {name}: [(u64, u16); {len(order)}] = [']
    text += [f'    (0x{h:016x}, {bits[h]}), // {lit_text(names[h])}' for h in order]
    text += ['];', '']
    return text


def write_table(path, profiles):
    if len(profiles) > 16:
        sys.exit('more than 16 profiles: --drop the oldest, or give the lists wider bits')
    text = [HEAD.rstrip('\n'), '']
    text.append(f'pub static PROFILES: [Profile; {len(profiles)}] = [')
    for k, q in enumerate(profiles):
        text.append(f'    Profile {{ id: "{q["id"]}", pgf: "{q["pgf"]}", latex: "{q["latex"]}", bp: BpRule::{q["bp"]}, guard: &GUARD_{k}, guard_any: &GUARD_ANY_{k}, absent: &ABSENT_{k}, registers: &REGISTERS_{k}, styles: &STYLES_{k}, latex_id: &LATEX_{k} }},')
    text += ['];', '']
    # What tells them apart.  A name that one of them does not guard has 0 there.
    guards = [dict(q['guard']) for q in profiles]
    told = lambda n: n.startswith('pgfk@')
    names = sorted({n for g in guards for n in g if not told(n) and len({g2.get(n) for g2 in guards}) > 1})
    pick = [(n, [g.get(n, '0' * 16) for g in guards]) for n in names]
    identities = [dict(q['identity']) for q in profiles]
    pick += [(n, [i.get(n, '0' * 16) for i in identities]) for n in sorted({n for i in identities for n in i}) if n not in names]
    text.append('/// What tells the profiles apart: a name, and the hash it has in each.  `Tex::native_profile_pick` takes the nearest, and then it is that profile that a diagram is held against, all of it.')
    text.append(f'pub static PICK: [(&[u8], [u64; {len(profiles)}]); {len(pick)}] = [')
    for n, hs in pick:
        text.append(f'    ({lit(n)}, [' + ', '.join(f'0x{h}' for h in hs) + ']),')
    text += ['];', '']
    for k, q in enumerate(profiles):
        text.append(f'// ---- {q["id"]}')
        text += row_list(f'GUARD_{k}', '(&[u8], u64)', [f'    ({lit(n)}, 0x{h}),' for n, h in q['guard']])
        text += row_list(f'GUARD_ANY_{k}', '&[u8]', [f'    {lit(n)},' for n in q['any']])
        text += row_list(f'ABSENT_{k}', '&[u8]', [f'    {lit(n)},' for n in q['absent']])
        text += row_list(f'REGISTERS_{k}', '(&[u8], i32)', [f'    ({lit(n)}, {v}),' for n, v in q['registers']])
        rows = []
        for style, entries in q['styles']:
            rows.append(f'    ({lit(style)}, &[')
            rows += [f'        ({lit(n)}, 0x{h}),' for n, h in entries]
            rows.append('    ]),')
        text += row_list(f'STYLES_{k}', '(&[u8], &[(&[u8], u64)])', rows, len(q['styles']))
        text += row_list(f'LATEX_{k}', '(&[u8], u64)', [f'    ({lit(n)}, 0x{h}),' for n, h in q['identity']])
    text += merged(profiles, 'keys', 'KEYS', "Keys of `/tikz` and tikz-cd's directory as TikZ, its libraries and tikz-cd define them, by hash of name and `.@cmd`, sorted.  What each is, is there to be read.")
    text += merged(profiles, 'safe', 'SAFE', 'Macros that may stay in a cell whatever they mention, by hash of name and body, sorted.')
    text += merged(profiles, 'known', 'KNOWN', "All macros as installed but PGF's and scratch, by the same hash, sorted.")
    for fn, name in (('is_stock_key', 'KEYS'), ('is_safe', 'SAFE'), ('is_known', 'KNOWN')):
        text += [f'pub fn {fn}(profile: i32, h: u64) -> bool {{', f'    matches!({name}.binary_search_by_key(&h, |e| e.0), Ok(k) if {name}[k].1 & 1 << profile != 0)', '}', '']
    with open(path, 'w', encoding='latin-1', errors='replace') as f:
        f.write('\n'.join(text))


def read_table(path):
    """The profiles a table holds, as write_table was given them."""
    with open(path, encoding='latin-1') as f:
        text = f.read()
    profiles = []
    for m in re.finditer(r'^    Profile \{ id: "([^"]*)", pgf: "([^"]*)", latex: "([^"]*)", bp: BpRule::(\w+),', text, re.M):
        profiles.append({'id': m.group(1), 'pgf': m.group(2), 'latex': m.group(3), 'bp': m.group(4), 'keys': {}, 'safe': {}, 'known': {}})
    if not profiles:
        return []

    def section(name):
        m = re.search(r'^static ' + re.escape(name) + r': \[[^\n]*\] = \[\n(.*?)^\];', text, re.M | re.S)
        return m.group(1).split('\n')[:-1] if m else []
    for k, q in enumerate(profiles):
        q['guard'] = [(unlit(a), b) for a, b in (re.fullmatch(r'    \(b"(.*)", 0x(\w+)\),', r).groups() for r in section(f'GUARD_{k}'))]
        q['identity'] = [(unlit(a), b) for a, b in (re.fullmatch(r'    \(b"(.*)", 0x(\w+)\),', r).groups() for r in section(f'LATEX_{k}'))]
        q['any'] = [unlit(re.fullmatch(r'    b"(.*)",', r).group(1)) for r in section(f'GUARD_ANY_{k}')]
        q['absent'] = [unlit(re.fullmatch(r'    b"(.*)",', r).group(1)) for r in section(f'ABSENT_{k}')]
        q['registers'] = [(unlit(a), int(b)) for a, b in (re.fullmatch(r'    \(b"(.*)", (-?\d+)\),', r).groups() for r in section(f'REGISTERS_{k}'))]
        styles = []
        for r in section(f'STYLES_{k}'):
            m = re.fullmatch(r'    \(b"(.*)", &\[', r)
            if m:
                styles.append((unlit(m.group(1)), []))
                continue
            m = re.fullmatch(r'        \(b"(.*)", 0x(\w+)\),', r)
            if m:
                styles[-1][1].append((unlit(m.group(1)), m.group(2)))
        q['styles'] = styles
    for what, name in (('keys', 'KEYS'), ('safe', 'SAFE'), ('known', 'KNOWN')):
        for h, b, n in (re.fullmatch(r'    \(0x(\w+), (\d+)\), // (.*)', r).groups() for r in section(name)):
            for k, q in enumerate(profiles):
                if int(b) >> k & 1:
                    q[what][int(h, 16)] = unlit(n)
    return profiles


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--engine', help='a build with the feature commute-probe')
    ap.add_argument('--check', action='store_true', help='say whether the table holds what this tree gives, write nothing')
    ap.add_argument('--name', help='what to call this profile, by default from the versions of PGF and LaTeX')
    ap.add_argument('--drop', help='take this profile out of the table')
    ap.add_argument('--list', action='store_true', help='say what the table holds')
    ap.add_argument('--rewrite', action='store_true', help='write the table again as this writes it, with what it holds')
    ap.add_argument('--out', default=OUT)
    args = ap.parse_args()
    if args.list or args.drop or args.rewrite:
        profiles = read_table(args.out)
        if args.rewrite:
            write_table(args.out, profiles)
        if args.drop:
            if not any(q['id'] == args.drop for q in profiles):
                sys.exit(f'no profile {args.drop} in {args.out}')
            profiles = [q for q in profiles if q['id'] != args.drop]
            if not profiles:
                sys.exit('the table has to hold a profile')
            write_table(args.out, profiles)
        for q in profiles:
            print(f"{q['id']}: PGF {q['pgf']}, LaTeX {q['latex']}, lengths by {q['bp']}, guard {len(q['guard'])}, safe {len(q['safe'])}, known {len(q['known'])}, keys {len(q['keys'])}")
        return 0
    if not args.engine:
        sys.exit('--engine is needed: a build with the feature commute-probe')
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
    # What the kept macros run of LaTeX's, and of the primitives, the same under every preamble: real PGF's own use of them is what the path leaves out.
    said = {}
    for _, _, n, ment in safe_dump(engine, STABLE[0], tmp, 'stable0'):
        said[n] = ment
    latex = sorted({m for n, _ in guard for m in said.get(n, []) if m in called and m in stock and m not in dict(guard) and m not in dynamic and m not in unstable and not m.startswith(('pgf', 'tikz')) and ':' not in m and not NOT_STATE.search(m) and not SCRATCH.match(m)})
    guard += [(n, stock[n]) for n in latex]
    prim_names = {n for n, _ in guard for n in OTHERS.get(n, ())}
    firsts = [PRIMITIVES.get(f'stable{k}', {}) for k in range(len(STABLE))]
    prims = sorted(n for n in prim_names if n in firsts[0] and all(f.get(n) == firsts[0][n] for f in firsts) and n not in NOT_GUARDED)
    guard += [(n, firsts[0][n]) for n in prims]
    print(f'of LaTeX {len(latex)}: ' + ' '.join(latex)[:1500])
    # What real tikz-cd calls and libraries define again: any of the forms that STABLE shows.
    guard_any = sorted(n for n in called if n in unstable and n.startswith(('pgf', 'tikz')) and n not in dynamic and not NOT_STATE.search(n) and n not in NOT_ANY)
    print(f'guarded by any form {len(guard_any)}: ' + ' '.join(guard_any)[:1500])
    any_hashes = {int(r[0], 16): r[2] for k in range(len(STABLE)) for r in safe_dump(engine, STABLE[k], tmp, f'stable{k}') if r[2] in set(guard_any)}
    print(f'primitives {len(prims)}')
    # Only the keys that nearly every diagram tries: a name that comes from one diagram's options is a key of that diagram, which the library knows or declines.  Those of a row or a column
    # are the engine's to ask for, by number.
    absent = sorted(n for n, seen in looked.items() if n not in stock and n not in dynamic and len(seen) >= len(docs) // 2 and all(h1 == '0' * 16 for _, h1 in seen)
                    and re.fullmatch(r'pgfk@/(tikz|tikz/commutative diagrams|pgf)/[^/]+/\.@cmd', n) and not re.search(r'/(column|row|\d+-row)[ /]|/[rdlu\']+/\.@cmd', n))
    print(f'guard {len(guard)}, absent {len(absent)}, keys {len(stock_keys)}')
    if args.check:
        print('\n'.join(absent))

    # What may stay in a cell.
    safe = set()
    known = {}
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
            known.update({int(h, 16): name for h, _, name, _ in rows if not name.startswith(('pgf', 'tikz', 'ifpgf', 'iftikz')) and not SCRATCH.match(name) and name not in UNSTEADY})
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
                if name in bad or name.rstrip(' ') in NOT_SAFE_THEMSELVES or name.startswith(('pgf', 'tikz', '__', 'g__', 'l__', 'c__')) or (':' in name and not name.startswith(('single:', 'active:'))):
                    continue
                if '@' in name and not name.startswith(AT_NAMES):
                    continue
                safe.add((h, name))
    for k in range(len(STABLE)):
        known.update({int(h, 16): name for h, _, name, _ in safe_dump(engine, STABLE[k], tmp, f'stable{k}') if not name.startswith(('pgf', 'tikz', 'ifpgf', 'iftikz')) and not SCRATCH.match(name) and name not in UNSTEADY})
    known.update(any_hashes)
    print(f'safe {len(safe)}, known {len(known)}')

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
    registers = sorted({n for n, k in names.items() if k == 'R'} | set(ALWAYS_REGISTERS))
    held.update(ALWAYS_REGISTERS)
    # And what those mention of PGF's and TikZ's, on to the end: real tikz-cd reaches much by `\\let\\tikz@next=` that no call shows by name.  What a real diagram or picture changes is scratch, whatever its name.
    reach = {n for n, _ in guard}
    todo = list(reach)
    while todo:
        for m in said.get(todo.pop(), []):
            if m.startswith(('pgf', 'tikz')) and m not in reach:
                reach.add(m)
                todo.append(m)
    leaves = picture_leaves(engine, tmp)
    have = dict(guard)
    scratch = lambda n: n in dynamic or n in leaves or n in common or TRACE.search(n) or UNREAD.search(n)
    further = sorted(n for n in reach if n not in have and n in stock and n not in unstable and not NOT_STATE.search(n) and not scratch(n) and n not in NOT_GUARDED)
    guard = sorted(guard + [(n, stock[n]) for n in further])
    print(f'reached from those: {len(reach)}, {len(further)} more to guard')
    # What only the words of NOT_STATE kept out, or libraries define again, and a real diagram leaves as it found: by any of the forms that STABLE shows.
    any_more = sorted(n for n in (called | reach) if n.startswith(('pgf', 'tikz')) and n in stock and n not in dict(guard) and n not in guard_any and not scratch(n) and n not in NOT_ANY and n not in NOT_GUARDED and not SCRATCH_OF_PICTURES.search(n))
    guard_any = sorted((set(guard_any) | set(any_more)) - NOT_GUARDED)
    known.update({int(r[0], 16): r[2] for k in range(len(STABLE)) for r in safe_dump(engine, STABLE[k], tmp, f'stable{k}') if r[2] in set(guard_any)})
    print(f'guarded by any form, now {len(guard_any)}: ' + ' '.join(guard_any)[:1500])
    print(f'{len(written)} settings of keys: {len(more)} macros and switches more to guard, {len(registers)} registers')
    print('  guard: ' + ' '.join(more)[:1500])
    print('  registers: ' + ' '.join(registers)[:1000])

    pgf, latex, identity, bp = tree_identity(engine, tmp)
    profile = {'id': args.name or f"pgf{pgf.split()[0]}-latex{latex.split()[0]}-pl{latex.split()[-1]}", 'pgf': pgf, 'latex': latex, 'bp': bp, 'identity': identity,
               'guard': [(n, h) for n, h in guard], 'any': list(guard_any), 'absent': list(absent), 'registers': [(n, held[n]) for n in registers], 'styles': [(st, list(e)) for st, e in styles],
               'keys': {int(h, 16): n for h, n in stock_keys}, 'safe': {int(h, 16): n for h, n in safe}, 'known': dict(known)}
    subprocess.run(['rm', '-rf', tmp])
    profiles = read_table(args.out) if os.path.exists(args.out) else []
    if args.check:
        same = next((q for q in profiles if q['id'] == profile['id']), None)
        if same is None:
            print(f"{args.out}: no profile {profile['id']}")
            return 1
        differ = [k for k in profile if same.get(k) != profile[k]]
        print(f"{args.out}: {profile['id']} " + ('is what this tree gives' if not differ else 'differs from what this tree gives: ' + ', '.join(differ)))
        return 0 if not differ else 1
    profiles = [q for q in profiles if q['id'] != profile['id']] + [profile]
    write_table(args.out, profiles)
    print(f"wrote {args.out}: {profile['id']}, {len(profiles)} profiles: " + ' '.join(q['id'] for q in profiles))
    print(f"PROFILE {profile['id']}")
    return 0


if __name__ == '__main__':
    sys.exit(main())
