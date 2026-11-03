#!/usr/bin/env -S python3 -I
r"""Generates crates/engine/src/native_table.rs: what the native diagram path knows about the installed tikz-cd, PGF and LaTeX.  DESIGN section 10.

    cargo build --release -p rstex --features rstex-engine/commute-probe --target-dir target/native/probe
    python3 -I tests/cd/native/gen_table.py --engine target/native/probe/release/rstex

Three lists, all from runs of the engine with the path off:

  GUARD    Macros and keys that real tikz-cd runs or reads, with their hash.  tests/cd is compiled, and the probe of src/native_probe.rs says which macros were called and which
           names of keys were looked up.  Kept are those of PGF, TikZ and tikz-cd that exist at \begin{document} and are the same at its end and under every preamble in STABLE.
           What a document changes then shows as a hash that differs, and another version of PGF or tikz-cd shows in hundreds of them.
  ABSENT   Names of keys that real tikz-cd asks for and that do not exist in a plain preamble, like `every text node part`: a document that defines one changes the diagrams.
  SAFE     Hashes of macros, name and body, that may stay in a cell without a look at what they mention: LaTeX's and those of the packages in SAFE_PREAMBLES, less what changes how text is read.
           A macro of the document, or of any other package, is walked instead, and a hash can't be had by a redefinition that says something else.

The output depends on the TeX tree, which is why the path is off on any other: the hashes fail, and a run gets slower and no worse.
"""
import argparse
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
SAFE_PACKAGES = 'amsmath,amssymb,amsfonts,amsthm,mathtools,xcolor,graphicx,stmaryrd,mathrsfs,bm,eucal,enumerate,dsfont,bbm,mathabx,wasysym,relsize,nicefrac,xfrac,cancel,bbold,upgreek,centernot,accents,esint,yfonts,tensor,mleftright,scalerel,bigdelim,multirow,array,slashed,braket,physics'.split(',')
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
             'ExplSyntaxOn', 'ExplSyntaxOff', 'usepackage', 'RequirePackage', 'documentclass', 'begin', 'end', 'end ', 'tikzcd', 'endtikzcd', 'par'}
# These mention such a thing only to load a font's description: nothing a cell reads is touched.
NOT_DANGEROUS = {'selectfont', 'pickup@font', 'define@newfont', 'try@load@fontshape', 'nfss@catcodes', 'DeclareFontShape', 'DeclareFontFamily', 'wrong@fontshape', 'maybe@load@fontshape', 'not@math@alphabet', 'use@mathgroup', 'mathversion', 'getanddefine@fonts', 'check@mathfonts', 'glb@settings'}
AT_NAMES = ('@if', '@first', '@second', '@third', '@gobble', '@protected', '@testopt', '@xargdef', '@empty', '@tempa', '@nil', 'protect', 'kernel@')
# Always kept: the real \end has to come to \endtikzcd, and these say which version is here.
ALWAYS = ['end', 'end ', '@checkend', 'ver@tikz-cd.sty', 'pgfversion']


def run(engine, first, cwd, env_extra=None):
    env = dict(os.environ, RSTEX_NO_SNAPSHOT='1', RSTEX_NO_COMMUTE='1', TEXINPUTS=CD + ':' + os.environ.get('TEXINPUTS', ''))
    p = subprocess.run([engine, '-progname=pdflatex', '-interaction=batchmode', '-jobname=x', first], cwd=cwd, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=600)
    return p.returncode


def installed(pkg, cls='sty'):
    return subprocess.run(['kpsewhich', f'{pkg}.{cls}'], capture_output=True).returncode == 0


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
    ap.add_argument('--check', action='store_true', help='say what would change, write nothing')
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
        # 16, 20 and 27 change keys for all that follows, on purpose: what changes there says nothing about the rest.
        changes = n[:2] in ('16', '20', '27', '17', '15', '25')
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

    # Their hash under each preamble.
    stock = None
    unstable = set()
    for k, pre in enumerate(STABLE):
        rows = {r[2]: r[0] for r in safe_dump(engine, pre, tmp, f'stable{k}')}
        if not rows:
            print(f'  preamble {k} gave nothing: {pre}', file=sys.stderr)
            continue
        if stock is None:
            stock = rows
        else:
            unstable |= {n for n, h in stock.items() if rows.get(n, h) != h}
    print(f'{len(stock)} macros in the plain preamble, {len(unstable)} of them differ under another preamble')

    wanted = {n for n in called if n.startswith(('pgf', 'tikz'))} | {n for n in looked if n in stock}
    guard = sorted((n, stock[n]) for n in wanted if n in stock and n not in dynamic and n not in unstable)
    guard += [(n, stock[n]) for n in ALWAYS if n in stock and n not in dict(guard)]
    # Only the keys that nearly every diagram tries: a name that comes from one diagram's options is a key of that diagram, which the library knows or declines.  Those of a row or a column
    # are the engine's to ask for, by number.
    absent = sorted(n for n, seen in looked.items() if n not in stock and n not in dynamic and len(seen) >= len(docs) // 2 and all(h1 == '0' * 16 for _, h1 in seen)
                    and re.fullmatch(r'pgfk@/(tikz|tikz/commutative diagrams|pgf)/[^/]+/\.@cmd', n) and not re.search(r'/(column|row|\d+-row)[ /]|/[rdlu\']+/\.@cmd', n))
    print(f'guard {len(guard)}, absent {len(absent)}')
    if args.check:
        print('\n'.join(absent))

    # What may stay in a cell.
    safe = set()
    for k, pre in enumerate(SAFE_PREAMBLES):
        pkgs = [p for p in SAFE_PACKAGES if installed(p)]
        # mathabx and bbold and the like redefine a great deal, and each may stop another from loading: on their own as well.
        for j, plist in enumerate([[p for p in pkgs if p in ('amsmath', 'amssymb', 'amsfonts', 'amsthm', 'mathtools', 'xcolor', 'graphicx', 'stmaryrd', 'mathrsfs', 'bm', 'eucal', 'enumerate', 'dsfont', 'bbm', 'array', 'multirow', 'slashed', 'cancel', 'nicefrac', 'relsize', 'upgreek', 'tensor', 'accents')],
                                   [p for p in pkgs if p in ('xfrac', 'wasysym', 'centernot', 'mleftright', 'scalerel', 'bigdelim', 'braket', 'physics', 'esint', 'yfonts')],
                                   [p for p in pkgs if p == 'mathabx'],
                                   [p for p in pkgs if p == 'bbold']]):
            if not plist:
                continue
            preamble = pre + ''.join(f'\\usepackage{{{p}}}' for p in plist)
            rows = safe_dump(engine, preamble, tmp, f'safe{k}-{j}')
            mentions = {}
            own = {}
            for h, flags, name, ment in rows:
                mentions[name] = ment
                own[name] = flags
            # Upward from what changes how text is read.
            bad = {n for n, f in own.items() if f} | set(DANGEROUS)
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
                if name in bad or name.startswith(('pgf', 'tikz', '__', 'g__', 'l__', 'c__')) or ':' in name or name in ('arrow', 'ar', 'rar', 'lar', 'dar', 'uar', 'urar', 'ular', 'drar', 'dlar'):
                    continue
                if '@' in name and not name.startswith(AT_NAMES):
                    continue
                safe.add((h, name))
    print(f'safe {len(safe)}')

    def lit(n):
        return 'b"' + n.encode('utf-8', 'surrogateescape').decode('latin-1').replace('\\', '\\\\').replace('"', '\\"') + '"'

    text = ['//! What the native diagram path knows without looking.  Generated by `tests/cd/native/gen_table.py`, which says what each list is.  Do not edit.', '']
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
    if not args.check:
        with open(OUT, 'w', encoding='latin-1', errors='replace') as f:
            f.write('\n'.join(text))
        print(f'wrote {OUT}: {sum(len(t) + 1 for t in text)} bytes')
    subprocess.run(['rm', '-rf', tmp])
    return 0


if __name__ == '__main__':
    sys.exit(main())
