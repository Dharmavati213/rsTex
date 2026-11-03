#!/usr/bin/env -S python3 -I
"""gen_manifest.py -- write tests/corpus/manifests/public.json from what fetch.py put in refs/corpus/.

    gen_manifest.py [--timings RESULTS] [--stacks-aux RESULTS]         RESULTS is the results.jsonl of a prepare run

Everything downloaded is read as data: no Makefile, Lua script or Python script from a download runs. build.lua and config-*.lua are
parsed with regular expressions for the handful of settings that decide how a test runs. Two system tools do read downloaded text,
under the same restrictions as any compile: weave on Knuth's .web files, and pdftex with docstrip on two .dtx/.ins sets. What it writes:

  refs/corpus/l3build/<suite>/       one flat test directory per l3build configuration, copied together as `l3build check` does
  refs/corpus/l3build-doc/<repo>/    one flat directory per module, as `l3build doc` has
  refs/corpus/stacks-book/           the whole-book master of the Stacks Project
  refs/corpus/rewritten/<tree>/      trees with `{../` removed from file arguments, which openin_any=p would refuse
  refs/corpus/generated/             woven Knuth sources, stub packages, docstrip output, and with --stacks-aux the .aux of the Stacks
                                     chapters and the HoTT book's main.labels, both from the oracle's own runs

To rebuild from nothing: fetch.py, gen_manifest.py, run.py prepare, gen_manifest.py --stacks-aux R --timings R, run.py prepare, once more
gen_manifest.py --timings R, and run.py prepare again. The second generation adds entries, the third moves failures into judge_failure.

Tiers are not chosen by looking at results; the rule is in the manifest description. --timings supplies only the time limit and
which entries a smoke test can start from, and it marks a document the oracle stops on with a TeX error as judge_failure.
"""
import argparse
import fnmatch
import glob
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(os.path.dirname(HERE)))
CORPUS = os.path.join(REPO, 'refs', 'corpus')
OUT = os.path.join(REPO, 'tests', 'corpus', 'manifests', 'public.json')
EPOCH_L3BUILD = '1463734800'
# l3build runs a test in its output directory, so a file the test writes can be found again by \IfFileExists, which goes through
# \pdffilesize and kpathsea.  Here the two directories differ, so the output directory goes on the search path.
L3ENV = {'SOURCE_DATE_EPOCH': EPOCH_L3BUILD, 'max_print_line': '9999', 'error_line': '79', 'half_error_line': '50', 'TEXINPUTS': '@OUT@:'}
STANDARD_PERCENT = {'public-latex2e-tests': 20, 'public-latex3-tests': 25, 'public-stacks': 15, 'public-package-tests': 20, 'public-latex2e-docs': 15, 'public-latex3-docs': 15}
SMOKE_PER_SOURCE = {'public-latex2e-tests': 2, 'public-latex3-tests': 2, 'public-stacks': 1}
UNPACK_MODULES = ('l3trial', 'xpackages')
TIME_LIMIT_MS = 20000        # a last pass over this goes to `full`, whatever the hash says
exclusions = []
STATED_BOUNDS = [
    {'what': 'TeX Live\'s own documentation tree', 'why': '/usr/share/texmf-dist/doc does not exist on this Arch install. tikz-cd-doc, pgfmanual, amsldoc, source2e, usrguide, the KOMA guide, the memoir manual and others come from their upstream sources at the installed versions instead'},
    {'what': 'latex3/latex3: docs of l3trial, l3leftovers and xpackages', 'why': 'Packages not installed in this TeX Live, so their documentation cannot build. Their test files are in the corpus, against a docstrip of the package'},
    {'what': 'ryankeleti/ega', 'why': 'A copy is local in ~/math, which belongs to the source that covers the owner\'s directories'},
    {'what': 'l3build tests run on the installed format', 'why': 'l3build builds a format from the tree with its own hyphen.cfg. The oracle\'s log on this installation is the reference, so a test that differs from its .tlg still counts. tests/corpus/public/check_normalize.py measures how many match'},
    {'what': 'Packages this TeX Live does not have', 'why': 'A document needing one stays oracle-fails with the reason, except libertine, libertinus, mtpro2, eulervm, mathdesign, fontawesome5, beramono, berasans, beraserif, fetamont, adforn, dantelogo and bbm: those only choose fonts, so refs/corpus/generated/stubs has a stand-in, and the entry notes say so'},
    {'what': 'Biber and .glo glossaries', 'why': 'Biber is not run, so biblatex documents have no bibliography. makeindex runs on .idx only, once, after pass 1, so a change history in a .dtx stays empty'},
    {'what': 'Trees rewritten to run under openin_any=p', 'why': 'cam-gant, ic-gant, hartshorne and debray-alggeo use {../ in file arguments. refs/corpus/rewritten has copies with that prefix removed'},
    {'what': 'The .tpf and .tix references of .pvt and .lit tests', 'why': 'Those tests are run and compared with the oracle like the rest, but no check here compares the oracle with those stored files'},
]


def rel(p):
    return os.path.relpath(p, REPO)


def hash_rank(s):
    return hashlib.sha256(s.encode()).hexdigest()


def tree(name):
    return os.path.join(CORPUS, name, 'tree')


# ---------------------------------------------------------------------------------------------------- l3build settings
def lua_text(path):
    try:
        with open(path, encoding='utf-8', errors='replace') as f:
            text = f.read()
    except OSError:
        return ''
    return re.sub(r'--[^\n]*', '', text)


def lua_string(text, var):
    m = re.search(rf'^\s*{var}\s*=\s*(?:{var}\s+or\s+)?"([^"]*)"', text, re.M)
    return m.group(1) if m else None


def lua_number(text, var):
    m = re.search(rf'^\s*{var}\s*=\s*(?:{var}\s+or\s+)?(\d+)', text, re.M)
    return int(m.group(1)) if m else None


def lua_list(text, var):
    m = re.search(rf'^\s*{var}\s*=\s*(?:{var}\s+or\s+)?\{{([^}}]*)\}}', text, re.M | re.S)
    return re.findall(r'"([^"]*)"', m.group(1)) if m else None


def lua_bool(text, var):
    m = re.search(rf'^\s*{var}\s*=\s*(true|false)', text, re.M)
    return m.group(1) == 'true' if m else None


def module_of(root, d):
    """The nearest directory above a test directory with a build.lua."""
    cur = d
    while cur != root:
        cur = os.path.dirname(cur)
        if os.path.isfile(os.path.join(cur, 'build.lua')):
            return cur
    return root


def settings(root, module, config):
    """l3build reads build.lua, which pulls in build-config.lua with `x = x or default`, and then the config file on top."""
    layers = [lua_text(config)] if config else []
    layers += [lua_text(os.path.join(module, 'build.lua')), lua_text(os.path.join(root, 'build-config.lua'))]

    def get(fn, var):
        for t in layers:
            v = fn(t, var)
            if v is not None:
                return v
        return None
    s = {'testfiledir': get(lua_string, 'testfiledir') or 'testfiles',
         'testsuppdir': get(lua_string, 'testsuppdir'),
         'checkruns': get(lua_number, 'checkruns') or 1,
         'checkengines': get(lua_list, 'checkengines') or ['pdftex', 'xetex', 'luatex'],
         'checkformat': get(lua_string, 'checkformat') or 'latex',
         'stdengine': get(lua_string, 'stdengine'),
         'checksuppfiles': get(lua_list, 'checksuppfiles') or [],
         'includetests': get(lua_list, 'includetests'),
         'excludetests': get(lua_list, 'excludetests') or []}
    s['stdengine'] = s['stdengine'] or s['checkengines'][0]
    s['testfiledir'] = os.path.normpath(s['testfiledir'])
    return s


def discover_l3build(name):
    """Every directory with test files, each with the settings of the configuration that names it."""
    root = tree(name)
    dirs = sorted({os.path.dirname(p) for ext in ('lvt', 'pvt', 'lit') for p in glob.glob(os.path.join(root, '**', '*.' + ext), recursive=True)})
    out = []
    for d in dirs:
        module = module_of(root, d)
        reltest = os.path.relpath(d, module)
        chosen, how = None, None
        for cfg in sorted(glob.glob(os.path.join(module, 'config-*.lua'))):
            st = settings(root, module, cfg)
            if st['testfiledir'] == reltest and not st.get('includetests'):
                chosen, how = st, os.path.basename(cfg)
                break
        if chosen is None:
            st = settings(root, module, None)
            chosen, how = st, 'build.lua' if st['testfiledir'] == reltest else 'no configuration names this directory; module defaults'
        out.append({'dir': d, 'module': module, 'settings': chosen, 'config': how})
    return root, out


def same_file(a, b):
    try:
        if os.path.getsize(a) != os.path.getsize(b):
            return False
        with open(a, 'rb') as x, open(b, 'rb') as y:
            return x.read() == y.read()
    except OSError:
        return False


def sync_files(wanted, dest):
    """Make dest hold exactly these files.  Nothing is rewritten when it is already right, so regenerating the manifest is safe
    while a compile runs and leaves every oracle closure hash alone."""
    wanted = {n: p for n, p in wanted.items() if os.path.isfile(p)}
    for name, p in wanted.items():
        d = os.path.join(dest, name)
        if not same_file(p, d):
            shutil.copy(p, d)
    for name in os.listdir(dest):
        d = os.path.join(dest, name)
        if name not in wanted and os.path.isfile(d):
            os.unlink(d)


def write_if_changed(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    try:
        with open(path, encoding='utf-8', errors='surrogateescape') as f:
            if f.read() == text:
                return
    except OSError:
        pass
    with open(path, 'w', encoding='utf-8', errors='surrogateescape') as f:
        f.write(text)


def unpack_module(root, module):
    """Packages that TeX Live does not ship, needed by their own tests: docstrip the module with the system pdftex in a directory of
    its own, under the same restrictions as any downloaded TeX.  Returns {file name: path} for what came out."""
    dest = os.path.join(CORPUS, 'generated', 'unpack', os.path.relpath(module, CORPUS).replace('/', '--'))
    os.makedirs(dest, exist_ok=True)
    env = {'PATH': os.environ['PATH'], 'HOME': os.environ.get('HOME', ''), 'openin_any': 'p', 'openout_any': 'p', 'TZ': 'UTC', 'LC_ALL': 'C'}
    made = {}
    inputs = {}
    for p in glob.glob(os.path.join(module, '*.dtx')) + glob.glob(os.path.join(module, '*.ins')):
        inputs[os.path.basename(p)] = p
    sync_files(inputs, dest) if not os.path.isfile(os.path.join(dest, '.unpacked')) else None
    if not os.path.isfile(os.path.join(dest, '.unpacked')):
        for ins in sorted(n for n in inputs if n.endswith('.ins')):
            subprocess.run(['pdftex', '-no-shell-escape', '-interaction=batchmode', ins], cwd=dest, env=env, timeout=120, capture_output=True, check=False)
        with open(os.path.join(dest, '.unpacked'), 'w') as f:
            f.write('done\n')
    for n in os.listdir(dest):
        if n.endswith(('.sty', '.cls', '.def', '.tex', '.lua', '.cfg')) and n not in inputs:
            made[n] = os.path.join(dest, n)
    return made


def assemble(root, suite_id, spec):
    """One flat directory, as `l3build check` makes in build/test: support files, test support files, the tests themselves."""
    dest = os.path.join(CORPUS, 'l3build', suite_id)
    os.makedirs(dest, exist_ok=True)
    st = spec['settings']
    wanted = {}
    support = os.path.join(root, 'support')
    for pat in st['checksuppfiles']:
        for p in glob.glob(os.path.join(support, pat)):
            wanted[os.path.basename(p)] = p
    texmf_top = os.path.join(root, 'texmf', 'tex', 'latex')
    for p in glob.glob(os.path.join(texmf_top, '*')):       # loose helper files; the package trees there would shadow the installation
        wanted[os.path.basename(p)] = p
    supp = st['testsuppdir']
    if supp:
        supp = os.path.normpath(os.path.join(spec['module'], supp))
    else:
        supp = os.path.join(spec['dir'], 'support')
    if os.path.isdir(supp):
        for p in glob.glob(os.path.join(supp, '*')):
            wanted[os.path.basename(p)] = p
            if os.path.isdir(p):                 # l3build's cp is `cp -RLf`: support directories come along
                shutil.copytree(p, os.path.join(dest, os.path.basename(p)), dirs_exist_ok=True)
    for p in glob.glob(os.path.join(spec['dir'], '*')):
        wanted[os.path.basename(p)] = p
    for rel_mod in UNPACK_MODULES:
        if os.path.normpath(spec['module']).startswith(os.path.join(root, rel_mod)):
            wanted.update(unpack_module(root, spec['module']))
    sync_files(wanted, dest)
    return dest


def l3build_entries(name, source):
    root, specs = discover_l3build(name)
    entries = []
    for spec in specs:
        st = spec['settings']
        rel_dir = os.path.relpath(spec['dir'], root)
        suite_id = name + '/' + rel_dir.replace('/', '--')
        tests = sorted(p for ext in ('lvt', 'pvt', 'lit') for p in glob.glob(os.path.join(spec['dir'], '*.' + ext)))
        engines = st['checkengines']
        if st['checkformat'] == 'context':
            exclusions.append({'what': f'{suite_id}: {len(tests)} test files', 'why': f'{spec["config"]} runs ConTeXt on LuaTeX only'})
            continue
        if 'pdftex' in engines:
            kw, engine = ({}, 'pdftex') if st['checkformat'] == 'latex' else ({'mode': 'plain'}, 'pdftex')
        elif 'etex' in engines:
            kw, engine = ({'fmt': 'latex', 'compare': 'log'}, 'etex') if st['checkformat'] == 'latex' else ({'mode': 'plain', 'fmt': 'etex', 'compare': 'log'}, 'etex')
        else:
            exclusions.append({'what': f'{suite_id}: {len(tests)} test files', 'why': f'{spec["config"]} lists only {", ".join(engines)}: the suite does not run these on the installed pdfTeX format, so here they would count errors, not engine behaviour'})
            continue
        dest = assemble(root, suite_id, spec)
        for t in tests:
            base = os.path.basename(t)
            stem = base.rsplit('.', 1)[0]
            e = {'id': f'l3build/{suite_id}/{stem}', 'source': source, 'main': rel(os.path.join(dest, base)),
                 'passes': 'auto', 'max_passes': st['checkruns'], 'env': L3ENV, 'log_normalize': 'l3build', 'gate_log': True, 'judge_failure': True, 'timeout': 120}
            e.update(kw)
            if b'DocumentMetadata' in open(t, 'rb').read():
                # tagpdf draws a UUID for its namespace from \\pdfuniformdeviate, which is seeded from the clock.
                e['prelude'] = '\\pdfsetrandomseed 42 '
            if spec['config'].startswith('no configuration'):
                e['notes'] = spec['config']
            entries.append(e)
    return entries


# ---------------------------------------------------------------------------------------------------- Stacks Project
def stacks_chapters():
    with open(os.path.join(tree('stacks'), 'Makefile'), encoding='utf-8', errors='replace') as f:
        text = f.read()
    m = re.search(r'^LIJST = ((?:[^\n]*\\\n)*[^\n]*)', text, re.M)
    return m.group(1).replace('\\\n', ' ').split() + ['fdl']


def stacks_parts(root, names):
    """Part titles: lines of chapters.tex with no backslash; a part starts at the next chapter named after it."""
    parts, pending = {}, None
    with open(os.path.join(root, 'chapters.tex'), encoding='utf-8') as f:
        for line in f:
            if '\\' not in line and line.strip():
                pending = line.strip()
            m = re.search(r'\\hyperref\[([a-z-]+)-section-phantom\]', line)
            if m and pending:
                parts[m.group(1)] = pending
                pending = None
    return parts


def stacks_book(commit):
    """The whole book, by the rules of the project's own make_book.py and make_index.py, reimplemented here. The date and the
    version are fixed strings, so the file depends only on the pinned commit."""
    root = tree('stacks')
    dest = os.path.join(CORPUS, 'stacks-book')
    os.makedirs(dest, exist_ok=True)
    names = stacks_chapters()
    keep = ('stacks-project-book.cls', 'my.bib', 'hyperref.cfg')
    for f in keep:
        if not same_file(os.path.join(root, f), os.path.join(dest, f)):
            shutil.copy(os.path.join(root, f), dest)
    L = []
    with open(os.path.join(root, 'preamble.tex'), encoding='utf-8') as f:
        for line in f:
            if line.startswith('%') or 'externaldocument' in line:
                continue
            if 'xr-hyper' in line:         # make_book.py turns this into CJKutf8, which this TeX Live does not have
                continue
            if line.startswith('\\IfFileExists{') or line.startswith('\\documentclass'):
                line = line.replace('amsart', 'amsbook').replace('stacks-project', 'stacks-project-book')
            L.append(line)
    L += ['\\begin{document}\n', '\\begin{titlepage}\n', '\\pagestyle{empty}\n', '\\setcounter{page}{1}\n',
          '\\centerline{\\LARGE\\bfseries Stacks Project}\n', '\\vskip1in\n', '\\noindent\n', f'\\centerline{{Version {commit[:7]}}}\n',
          '\\vskip1in\n', '\\noindent\n', 'The following people have contributed to this work:\n']
    people = []
    with open(os.path.join(root, 'CONTRIBUTORS'), encoding='utf-8') as f:
        for line in f:
            if line.startswith('%') or not line.strip():
                continue
            people.append(re.sub(r'\s*\([^)]*\)', '', line.rstrip()))     # the names in CJK script would need CJKutf8
    L += [', '.join(people) + '.\n', '\\end{titlepage}\n']
    with open(os.path.join(root, 'introduction.tex'), encoding='utf-8') as f:
        inside = False
        for line in f:
            inside = inside or line.startswith('\\begin{verbatim}')
            if inside:
                L.append(line)
            if line.startswith('\\end{verbatim}'):
                inside = False
    parts = stacks_parts(root, names)
    index = stacks_index(root, names)
    labels = ('definition', 'lemma', 'proposition', 'theorem', 'remark', 'remarks', 'example', 'exercise', 'situation', 'equation',
              'section', 'subsection', 'subsubsection', 'item')
    with open(os.path.join(root, 'chapters.tex'), encoding='utf-8') as f:
        chapters = f.read()
    for name in names + ['index']:
        if name in parts:
            L.append('\\part{' + parts[name] + '}\n')
        if name == 'index':
            lines = index
        else:
            with open(os.path.join(root, name + '.tex'), encoding='utf-8') as f:
                lines = f.readlines()
        verbatim = False
        for line in lines:
            if line.startswith('\\begin{verbatim}'):
                verbatim = True
            if verbatim:
                if line.startswith('\\end{verbatim}'):
                    verbatim = False
                if name != 'introduction':
                    L.append(line)
                continue
            if line.startswith(('\\input{preamble}', '\\begin{document}', '\\maketitle', '\\tableofcontents', '\\input{chapters}',
                                '\\bibliography', '\\end{document}')):
                continue
            if line.startswith('\\title{'):
                line = line.replace('\\title{', '\\chapter{')
            if '\\label{' in line:
                line = line.replace('\\label{', '\\label{' + name + '-')
            if '\\ref{' in line:
                for lab in labels:
                    line = line.replace('\\ref{' + lab + '-', '\\ref{' + name + '-' + lab + '-')
            L.append(line)
        L.append(chapters)
    L += ['\\bibliography{my}\n', '\\bibliographystyle{amsalpha}\n', '\\end{document}\n']
    book = os.path.join(dest, 'book.tex')
    write_if_changed(book, ''.join(L))
    return book


def find_clause(text, spot):
    depth = 0
    while True:
        spot += 1
        if text[spot] == '{':
            depth += 1
        elif text[spot] == '}':
            depth -= 1
            if depth < 0:
                return spot


def stacks_index(root, names):
    """The generated `Auto generated index` chapter: every {\\it term} of every definition environment."""
    terms, defs, titles = [], [], []
    for name in names:
        have_title, inside, text = False, False, ''
        with open(os.path.join(root, name + '.tex'), encoding='utf-8') as f:
            for line in f:
                if not have_title and '\\title{' in line:
                    n = line.find('\\title{') + 6
                    titles.append(line[n + 1:find_clause(line, n)])
                    have_title = True
                if inside:
                    text += ' ' + line.rstrip()
                    if line.startswith('\\end{definition}'):
                        inside = False
                        n = text.find('\\label{')
                        label = text[n + 7:find_clause(text, n + 6)] if n >= 0 else ''
                        label = name + '-' + label
                        found, k = [], text.find('{\\it ')
                        while k >= 0:
                            m = find_clause(text, k)
                            found.append(text[k:m + 1])
                            k = text.find('{\\it ', m)
                        terms += [(t, label) for t in found]
                        defs.append((found, label))
                elif line.startswith('\\begin{definition}'):
                    inside, text = True, line.rstrip()
    L = ['\\input{preamble}\n', '\\begin{document}\n', '\\title{Auto generated index}\n', '\\maketitle\n', '\n', '\\phantomsection\n',
         '\\label{section-phantom}\n', '\n', '\\tableofcontents\n', '\n', '\\frenchspacing\n', '\n', '\n',
         '\\begin{multicols}{2}[\\section{Alphabetized definitions}\\label{section-alphabetized}]\n']
    for t, lab in sorted(terms, key=lambda x: x[0].lower()):
        L += ['\\noindent\n', t + '\n', 'in \\ref{' + lab + '}\n', '\n']
    L += ['\\end{multicols}\n', '\n', '\\begin{multicols}{2}[\\section{Definitions listed per chapter}\\label{section-per-chapter}]\n']
    n = 0
    for name, title in zip(names, titles):
        L += ['\n', '\\medskip\\noindent\n', '{\\bf ' + title + '}\n', '\n', '\\medskip\n']
        while n < len(defs) and defs[n][1].startswith(name + '-definition'):
            L += ['\n', '\\noindent\n', 'In \\ref{' + defs[n][1] + '}: \n', ',\n'.join(defs[n][0]) + '\n', '\n']
            n += 1
    L += ['\\end{multicols}\n', '\n', '\\input{chapters}\n', '\\end{document}\n']
    return L


def stacks_entries(lock):
    root = tree('stacks')
    entries = []
    for name in stacks_chapters():
        e = {'id': f'stacks/{name}', 'source': 'public-stacks', 'main': rel(os.path.join(root, name + '.tex')), 'bib': 'auto', 'timeout': 900}
        entries.append(e)
    # The way the project really builds its chapters: every chapter reads the .aux of all of them through xr-hyper, so cross-chapter
    # references resolve and each chapter's hash table holds the labels of the whole work.  Those files come from the oracle's own chapter
    # runs, copied here once by --stacks-aux.
    aux_dir = os.path.join(CORPUS, 'generated', 'stacks-aux')
    if glob.glob(os.path.join(aux_dir, '*.aux')):
        for name in stacks_chapters():
            entries.append({'id': f'stacks-xr/{name}', 'source': 'public-stacks', 'tier': 'full', 'main': rel(os.path.join(root, name + '.tex')), 'bib': 'auto',
                            'timeout': 900, 'texinputs': [rel(aux_dir)], 'notes': 'With the .aux of every chapter on the search path, from the oracle run of stacks/*'})
    else:
        exclusions.append({'what': 'stacks-xr/*', 'why': 'refs/corpus/generated/stacks-aux is empty; run gen_manifest.py --stacks-aux RESULTS after preparing stacks/*'})
    book = stacks_book(lock['stacks']['commit'])
    entries.append({'id': 'stacks/book', 'source': 'public-stacks', 'tier': 'full', 'main': rel(book), 'bib': 'auto', 'timeout': 3000,
                    'notes': 'Whole-book master from tests/corpus/public/gen_manifest.py, by the rules of scripts/make_book.py, which was not run. No CJKutf8 (not installed), so contributor names in CJK script are left out'})
    return entries


# ---------------------------------------------------------------------------------------------------- open documents
_ROOT_CLASS = re.compile(r'\\document(?:class|style)\s*(\[[^\]]*\])?\s*\{([^}]*)\}')
_BEGIN_DOC = re.compile(r'\\begin\s*\{document\}')
_COMMENT = re.compile(r'(?m)(?<!\\)%.*$')


def read_text(p, limit=400000):
    try:
        with open(p, 'rb') as f:
            return _COMMENT.sub('', f.read(limit).decode('latin-1'))
    except OSError:
        return ''


def is_root(p):
    """A file you can hand to pdflatex.  A `subfiles` fragment is not: it names its parent with a path that has `..` in it, which
    openin_any=p refuses, and its text is typeset anyway by the master that includes it."""
    t = read_text(p)
    m = _ROOT_CLASS.search(t)
    if not m or not _BEGIN_DOC.search(t):
        return None
    return 'subfiles' if m.group(2).strip() == 'subfiles' else 'root'


def walk_files(d, exts):
    for cur, sub, names in os.walk(d):
        sub[:] = sorted(x for x in sub if not x.startswith('.'))
        for n in sorted(names):
            if n.endswith(exts):
                yield os.path.join(cur, n)


def rewrite_dotdot(name):
    """A copy of a tree with `{../` in file arguments turned into `{`.  The original would need openin_any above `p`, which the rules
    here do not allow for downloads. The search path gets the top of the copy, so the same files are found, and the text and
    macros do not change.  Only .tex, .sty and .cls are rewritten."""
    src = tree(name)
    dest = os.path.join(CORPUS, 'rewritten', name)
    seen = set()
    for cur, sub, names in os.walk(src):
        sub[:] = [x for x in sub if not x.startswith('.')]
        for n in names:
            p = os.path.join(cur, n)
            relp = os.path.relpath(p, src)
            d = os.path.join(dest, relp)
            seen.add(d)
            if not os.path.isfile(p):
                continue
            if n.endswith(('.tex', '.sty', '.cls')):
                with open(p, 'rb') as f:
                    data = f.read()
                new = re.sub(rb'\{(?:\.\./)+', b'{', data)
                os.makedirs(os.path.dirname(d), exist_ok=True)
                try:
                    with open(d, 'rb') as f:
                        if f.read() == new:
                            continue
                except OSError:
                    pass
                with open(d, 'wb') as f:
                    f.write(new)
            elif not same_file(p, d):
                os.makedirs(os.path.dirname(d), exist_ok=True)
                shutil.copy(p, d)
    return dest


def doc_id(prefix, root, main):
    return prefix + '/' + re.sub(r'[^A-Za-z0-9._/+-]+', '_', os.path.splitext(os.path.relpath(main, root))[0])


def add_doc(entries, source, prefix, root, main, cwd=None, **kw):
    cwd = cwd or os.path.dirname(main)
    e = {'id': doc_id(prefix, root, main), 'source': source, 'main': rel(main), 'cwd': rel(cwd), 'bib': 'auto', 'index': 'makeindex', 'timeout': 1200}
    e.update(kw)
    with open(main, 'rb') as f:
        if b'DocumentMetadata' in f.read():      # tagpdf draws UUIDs from \\pdfuniformdeviate, seeded from the clock
            e['prelude'] = '\\pdfsetrandomseed 42 '
    entries.append(e)
    return e


def body_entries(name, source, roots=None, at_top=False, rewrite=False, exclude=(), covered_elsewhere=False, exclude_why=None, more_stubs=(), **kw):
    """`roots` globs relative to the tree, or every file that is_root accepts.  Fragments are counted, and said to be left out."""
    root = rewrite_dotdot(name) if rewrite else tree(name)
    BODY_TREES[name] = root
    entries, fragments = [], 0
    if roots:
        mains = sorted(p for g in roots for p in glob.glob(os.path.join(root, g), recursive=True) if os.path.isfile(p))
    else:
        mains = []
        for p in walk_files(root, ('.tex', '.ltx')):
            kind = is_root(p)
            if kind == 'root':
                mains.append(p)
            elif kind == 'subfiles':
                fragments += 1
    stubs = sorted(set(tree_stubs(root)) | set(more_stubs))
    excluded = 0
    for main in mains:
        relmain = os.path.relpath(main, root)
        if any(fnmatch.fnmatch(relmain, x) for x in exclude):
            excluded += 1
            continue
        extra = dict(kw)
        if rewrite:
            extra['texinputs'] = [rel(root)] + list(extra.get('texinputs', []))
            extra['notes'] = 'Copy in refs/corpus/rewritten with {../ path prefixes removed, because openin_any=p refuses them'
        if stubs:
            extra['texinputs'] = list(extra.get('texinputs', [])) + [rel(stub_dir())]
            extra['notes'] = (extra.get('notes', '') + ' Runs with stub packages for ' + ', '.join(stubs) + ', which are not installed.').strip()
        add_doc(entries, source, f'{source[7:]}/{name}', root, main, cwd=root if at_top else None, **extra)
    if excluded and not covered_elsewhere:
        exclusions.append({'what': f'{name}: {excluded} files matching {", ".join(exclude)}', 'why': exclude_why or 'Not documents to run by themselves: the project says so, or they are drivers for a sibling file'})
    return entries


# Packages this TeX Live does not have, which only choose fonts.  Without them a document dies in its preamble and proves nothing; with
# a stub it runs on the default fonts, on both sides alike.  They are only on the search path of trees that load them, and they
# cannot shadow anything, since none of these is installed.
STUBS = {'libertine': '', 'libertinus': '', 'mtpro2': '', 'eulervm': '', 'mathdesign': '', 'fontawesome5': '', 'beramono': '', 'berasans': '', 'beraserif': '', 'fetamont': '',
         'adforn': '\\newcommand\\adforn[1]{}\n',
         'dantelogo': '\\newcommand\\DANTE{DANTE}\n',
         'bbm': '\\RequirePackage{amsfonts}\n\\newcommand\\mathbbm[1]{\\mathbb{#1}}\n\\newcommand\\mathbbmss[1]{\\mathbb{#1}}\n\\newcommand\\mathbbmtt[1]{\\mathbb{#1}}\n'}
_USES = re.compile(r'\\(?:usepackage|RequirePackage)\s*(?:\[[^\]]*\])?\s*\{([^}]*)\}', re.S)


def stub_dir():
    for name, body in STUBS.items():
        generated('stubs', name + '.sty', f'% stub for rstex corpus runs: the real package is not in this TeX Live\n\\ProvidesPackage{{{name}}}[2000/01/01 stub]\n\\DeclareOption*{{}}\n\\ProcessOptions\\relax\n{body}')
    generated('stubs', 'myinfo.tex', '% stand-in for the personal file that notes_d.cls reads\n')
    return os.path.join(CORPUS, 'generated', 'stubs')


def tree_stubs(root):
    used = set()
    for p in walk_files(root, ('.tex', '.sty', '.cls', '.dtx')):
        for m in _USES.finditer(read_text(p)):
            used |= {x.strip() for x in m.group(1).split(',')} & set(STUBS)
    return sorted(used)


BODY_TREES = {}


def state_unused_roots(entries):
    """Every file in a body tree that looks runnable and is not an entry, counted per tree, so that nothing is dropped silently."""
    used = {e.get('main') for e in entries}
    for name, root in sorted(BODY_TREES.items()):
        sub, plain = 0, 0
        for p in walk_files(root, ('.tex', '.ltx')):
            if rel(p) in used:
                continue
            kind = is_root(p)
            sub += kind == 'subfiles'
            plain += kind == 'root'
        if sub:
            exclusions.append({'what': f'{name}: {sub} `subfiles` fragments', 'why': 'They name a parent through `..`, refused by openin_any=p, and the master that is in the corpus typesets their text'})
        if plain:
            exclusions.append({'what': f'{name}: {plain} other files that have documentclass and begin-document', 'why': 'Not selected: examples, templates, unfinished material or drivers that the project itself does not build as documents'})


def generated(name, filename, text):
    path = os.path.join(CORPUS, 'generated', name, filename)
    write_if_changed(path, text)
    return path


def module_docs(name, source, modules, drop=()):
    """`l3build doc` for one repository, as data.  Each module gets one flat directory with its sources, its doc/ files and the shared
    configuration, as l3build's typeset directory has, and every .dtx and every root .tex there is an entry."""
    root = tree(name)
    entries = []
    for mod in modules:
        mdir = os.path.join(root, mod)
        if not os.path.isdir(mdir):
            continue
        dest = os.path.join(CORPUS, 'l3build-doc', name, mod.replace('/', '--'))
        os.makedirs(dest, exist_ok=True)
        wanted = {}
        for p in glob.glob(os.path.join(root, 'support', '*.cfg')) + glob.glob(os.path.join(root, 'support', '*.ist')):
            wanted[os.path.basename(p)] = p
        wanted.pop('fonttext.cfg', None)
        wanted.pop('hyphen.cfg', None)
        wanted.pop('texsys.cfg', None)
        wanted.pop('regression-test.cfg', None)
        for sub in ('', 'doc'):
            for p in glob.glob(os.path.join(mdir, sub, '*')):
                if os.path.isfile(p):
                    wanted[os.path.basename(p)] = p
        for p in glob.glob(os.path.join(root, 'texmf', 'makeindex', '*.ist')):
            wanted[os.path.basename(p)] = p
        sync_files(wanted, dest)
        for n in sorted(os.listdir(dest)):
            p = os.path.join(dest, n)
            if n.endswith('.dtx') or (n.endswith('.tex') and is_root(p) == 'root'):
                stem = n.rsplit('.', 1)[0]
                if stem in drop:
                    continue
                style = 'source2e.ist' if stem == 'source2e' and os.path.isfile(os.path.join(dest, 'source2e.ist')) else 'gind.ist'
                e = add_doc(entries, source, f'docs/{name}/{mod.replace("/", "--")}', dest, p, max_passes=4, timeout=1800, index_style=style)
                e['id'] = f'docs/{name}/{mod.replace("/", "--")}/{n}'
    return entries


def knuth_entries():
    """Knuth's distribution, in plain pdfTeX.  Every .web there is woven with the system weave, which only reads and writes text."""
    root = tree('knuth')
    source = 'public-knuth'
    entries = []
    woven = os.path.join(CORPUS, 'generated', 'knuth-woven')
    os.makedirs(woven, exist_ok=True)
    webs = sorted(p for p in glob.glob(os.path.join(root, '*', '*.web')))
    for web in webs:
        stem = os.path.basename(web)[:-4]
        out = os.path.join(woven, stem + '.tex')
        if not os.path.isfile(out):
            subprocess.run(['weave', web, out], cwd=woven, check=True, capture_output=True, env={'PATH': os.environ['PATH'], 'HOME': os.environ.get('HOME', '')})
        # webmac reads back CONTENTS.tex, which it wrote, by the name CONTENTS: only a search path can add the .tex in the output directory
        entries.append({'id': f'knuth/woven/{stem}', 'source': source, 'mode': 'plain', 'main': rel(out), 'timeout': 900, 'env': {'TEXINPUTS': '@OUT@:'},
                        'notes': f'{stem}.web from Knuth\'s distribution, woven by the system weave'})
    for relp in ('tex/texbook.tex', 'mf/mfbook.tex'):
        # The first lines of both files: "The file is distributed only for people to see its examples of TeX input" and an endless
        # \\errmessage{This manual is copyrighted and should not be TeXed}.  Taking that loop out is not ours to decide.
        entries.append({'id': 'knuth/' + relp[:-4], 'source': source, 'mode': 'plain', 'tier': 'full', 'main': rel(os.path.join(root, relp)),
                        'excluded': 'The file forbids being typeset, with a copyright notice and an endless \\errmessage in its first lines; the guard was left in place'})
    entries.append({'id': 'knuth/lib/story', 'source': source, 'mode': 'plain', 'first_line': '\\input story \\bye', 'jobname': 'story', 'cwd': rel(os.path.join(root, 'lib')), 'timeout': 300,
                    'notes': 'story.tex has no \\bye of its own, it is meant to be run this way'})
    for relp in ('tex/tripman.tex', 'mf/trapman.tex', 'web/webman.tex', 'errata/errorlog.tex', 'errata/errata.tex', 'tex/trip.tex'):
        p = os.path.join(root, relp)
        if not os.path.isfile(p):
            exclusions.append({'what': f'knuth: {relp}', 'why': 'not in the downloaded distribution'})
            continue
        e = {'id': 'knuth/' + relp[:-4], 'source': source, 'mode': 'plain', 'main': rel(p), 'timeout': 900}
        if relp == 'tex/trip.tex':
            e['notes'] = 'Not the real TRIP test, which has its own driver: this is trip.tex fed to a normal pdfTeX format, so most of it is error recovery'
        entries.append(e)
    return entries


def document_entries():
    E = []
    # Algebraic geometry and category theory first: these look like what the owner of this corpus writes.
    version = generated('hott', 'version.tex', '\\newcommand{\\OPTversion}{578b85c}\n')
    E += body_entries('hott', 'public-books', roots=['hott-*.tex', 'cover-*.tex'], seed_files={'version.tex': rel(version)}, timeout=1800)
    labels = os.path.join(CORPUS, 'generated', 'hott', 'main.labels')       # the Makefile's: cat *.aux | grep ^.newlabel
    if os.path.isfile(labels):
        E += body_entries('hott', 'public-books', roots=['exercise_solutions.tex', 'errata.tex'], seed_files={'version.tex': rel(version), 'main.labels': rel(labels)}, timeout=1800)
    else:
        exclusions.append({'what': 'hott: errata, exercise_solutions', 'why': 'They read main.labels; run gen_manifest.py --stacks-aux RESULTS after preparing books/hott/hott-online'})
    E += body_entries('cring', 'public-books', roots=['CRing.tex', 'standalone/*.tex'], at_top=True, timeout=1800)
    E += body_entries('cam-notes', 'public-notes', exclude=['archive/*'], covered_elsewhere=True)
    E += body_entries('cam-notes', 'public-notes', roots=['archive/*.tex'], texinputs=[rel(tree('cam-notes'))])
    E += body_entries('internal-methods', 'public-notes', at_top=True)
    E += body_entries('scenic', 'public-books', roots=['geom_intro.tex'])
    E += body_entries('cam-gant', 'public-notes', rewrite=True, roots=['*/*.tex'], exclude=['style/*'])
    E += body_entries('ic-gant', 'public-notes', rewrite=True, roots=['*/*.tex'], exclude=['style/*'])
    E += body_entries('geraschenko-ag', 'public-notes', roots=['AlgebraicGeometryNotes.tex'])
    E += body_entries('geraschenko-ha', 'public-notes', roots=['Halg.tex'])
    E += body_entries('debray-alggeo', 'public-notes', rewrite=True, roots=['m392c_Raskin_AG_notes.tex'], texinputs=[rel(os.path.join(tree('debray-style'), 'lecture_notes'))], more_stubs=['adforn'])
    E += body_entries('autour-dag', 'public-notes', roots=['notes.tex'])
    E += body_entries('dagnotes', 'public-notes', roots=['main.tex'])
    E += body_entries('caagnotes', 'public-notes', roots=['fa24.tex', 'sp25.tex'])
    E += body_entries('complex-ag-dg', 'public-notes', roots=['main.tex'])
    E += body_entries('acs-category', 'public-notes', roots=['acs-category-theory-notes.tex'])
    E += body_entries('matsumura', 'public-books', roots=['main.tex'])
    E += body_entries('jakpal-commalg', 'public-notes', roots=['commalg.tex'])
    E += body_entries('hartshorne', 'public-notes', rewrite=True)
    E += body_entries('napkin', 'public-books', roots=['Napkin.tex', 'flowchart-only.tex'], timeout=1800)
    E += body_entries('openlogic', 'public-books', roots=['open-logic-*.tex', 'courses/sample/*.tex'], timeout=1800)
    # Manuals.  TeX Live's doc tree is not installed here, so these come from the upstream repositories at the installed versions.
    E += module_docs('latex2e', 'public-latex2e-docs', ['base', 'required/amsmath', 'required/graphics', 'required/tools', 'required/cyrillic', 'required/firstaid', 'required/latex-lab'])
    E += module_docs('latex3', 'public-latex3-docs', ['l3kernel', 'l3backend', 'l3packages/l3keys2e', 'l3packages/xfp', 'l3packages/xparse', 'l3packages/xtemplate', 'l3experimental/l3draw', 'l3experimental/xcoffins', 'l3experimental/xgalley', 'articles', 'discussionpapers'])
    E += body_entries('pgf', 'public-manuals', roots=['doc/generic/pgf/pgfmanual.tex'], timeout=3000, max_passes=4)
    E += body_entries('tikz-cd', 'public-manuals', roots=['tikz-cd-doc.tex', 'tests/test-latex.tex'])
    E += body_entries('beamer', 'public-manuals', roots=['doc/beameruserguide.tex'], timeout=1800, max_passes=4)
    E += body_entries('hyperref', 'public-manuals', roots=['doc/hyperref-doc.tex'], timeout=1800, max_passes=4, index_style='gind.ist', texinputs=[rel(tree('hyperref'))])
    E += body_entries('hyperref', 'public-manuals', roots=['*.dtx'], timeout=1800, max_passes=4, index_style='gind.ist')
    E += body_entries('microtype', 'public-manuals', roots=['*.dtx'], timeout=900, max_passes=4, index_style='gind.ist')
    other = ['*_lualatex.tex', '*_luatex.tex', '*_xelatex.tex', '*_xetex.tex', '*_dvilualatex.tex', '*_context.tex', '*_dvilatex.tex']
    E += body_entries('microtype', 'public-manuals', roots=['testsuite/*.tex'], timeout=300, bib='none', index='none', exclude=other + ['*_pdftex.tex'], covered_elsewhere=False,
                      exclude_why='Named for another engine by the microtype test suite (the _pdftex ones do run, as plain pdfTeX, under their own entries)')
    E += body_entries('microtype', 'public-manuals', roots=['testsuite/*_pdftex.tex'], timeout=300, bib='none', index='none', mode='plain')
    E += body_entries('siunitx', 'public-manuals', roots=['siunitx.dtx', 'siunitx-code.tex', '*.tex'], at_top=False, timeout=1800, max_passes=4, index_style='gind.ist')
    E += body_entries('lshort', 'public-manuals', roots=['book/src/lshort-screen.tex', 'book/src/lshort-a5.tex', 'book/src/lshort-letter.tex', 'book/src/lshort.tex'], max_passes=4, index_style='lshort.ist')
    E += body_entries('lshort', 'public-manuals', roots=['beamer/presentation.tex', 'beamer/handouts.tex', 'intro/intro.tex', 'intro/intro.en.tex', 'sample/*.tex'])
    E += body_entries('memoir', 'public-manuals', roots=['doc-src/memman.tex'], max_passes=4, timeout=1800, index_style='memman.ist')
    E += body_entries('memoir', 'public-manuals', roots=['memoir.dtx', 'mempatch.dtx'], max_passes=4, timeout=1800, index_style='gind.ist')
    # The guide lists letter examples that scrlttr2-examples.dtx generates: docstrip them with the system pdftex, as unpack_module does.
    letters = os.path.join(CORPUS, 'generated', 'koma-letters')
    os.makedirs(letters, exist_ok=True)
    if not glob.glob(os.path.join(letters, 'letter-example-*')):
        dtx = 'scrlttr2-examples.dtx'
        shutil.copy(os.path.join(tree('koma-script'), 'doc', dtx), letters)
        subprocess.run(['pdftex', '-no-shell-escape', '-interaction=batchmode', dtx], cwd=letters, timeout=120, capture_output=True, check=False,
                       env={'PATH': os.environ['PATH'], 'HOME': os.environ.get('HOME', ''), 'openin_any': 'p', 'openout_any': 'p', 'TZ': 'UTC', 'LC_ALL': 'C'})
    E += body_entries('koma-script', 'public-manuals', roots=['doc/scrguide-en.tex', 'doc/scrguide-de.tex'], timeout=1800, max_passes=4, index_style='scrguide.ist', texinputs=[rel(letters)])
    E += body_entries('koma-script', 'public-manuals', roots=['koma-script-source-doc.dtx', 'scr*.dtx', 'tocbasic.dtx', 'japanlco.dtx', 'doc/scrlttr2-examples.dtx'], timeout=1800, max_passes=4, index_style='gind.ist')
    E += knuth_entries()
    # dedupe: a root found by two globs, and siunitx-code.tex above
    seen, out = set(), []
    for e in E:
        if e['id'] not in seen:
            seen.add(e['id'])
            out.append(e)
    return out


# ---------------------------------------------------------------------------------------------------- tiers
def assign_tiers(entries, timings):
    by_source = {}
    for e in entries:
        by_source.setdefault(e['source'], []).append(e)
    for source, es in by_source.items():
        ranked = sorted(es, key=lambda e: hash_rank(e['id']))
        pct = STANDARD_PERCENT.get(source, 40)
        want_smoke = SMOKE_PER_SOURCE.get(source, 1)
        smoke = 0
        for i, e in enumerate(ranked):
            if e.get('tier'):
                continue
            ms = timings.get(e['id'])
            slow = ms is not None and ms > TIME_LIMIT_MS
            sample = int(hash_rank(e['id'])[:8], 16) % 100 < pct
            fast_enough = ms is not None and ms < 2000
            if smoke < want_smoke and fast_enough and e['id'] in NORMAL:
                e['tier'] = 'smoke'
                smoke += 1
            elif sample and not slow:
                e['tier'] = 'standard'
            else:
                e['tier'] = 'full'


NORMAL = set()
DEEP_FAILURES = set()       # the oracle stops on a TeX error, not on a missing file: how it fails is behaviour to compare
NOT_DEEP = ('not found', "can't find file", 'not loadable', 'Unknown option', 'timeout', 'signal', 'fontspec')    # dies in its preamble: a pass on these would prove nothing


def load_timings(path):
    """Last-pass milliseconds per entry from a prepare run.  Only smoke and the time limit use this; no result decides anything else."""
    res = {}
    if path:
        with open(path) as f:
            for line in f:
                r = json.loads(line)
                if r.get('pass_ms'):
                    res[r['id']] = r['pass_ms'][-1]
                if r.get('class') == 'normal':
                    NORMAL.add(r['id'])
                reason = r.get('judged_failure') or (r.get('reason') if r.get('class') == 'oracle-fails' else '') or ''
                if reason and not any(x in reason for x in NOT_DEEP):
                    DEEP_FAILURES.add(r['id'])
    return res


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--timings', help='results.jsonl of a prepare run, for the time limit on tiers')
    ap.add_argument('--stacks-aux', help='results.jsonl of a prepare run: derive what the build systems would make from a first run, namely the Stacks chapters\' .aux and the main.labels of the HoTT book, into refs/corpus/generated/')
    args = ap.parse_args()
    if args.stacks_aux:
        dest = os.path.join(CORPUS, 'generated', 'stacks-aux')
        os.makedirs(dest, exist_ok=True)
        with open(args.stacks_aux) as f:
            for line in f:
                r = json.loads(line)
                if r['id'].startswith('stacks/') and r['id'] != 'stacks/book' and r.get('dir'):
                    p = os.path.join(r['dir'], 'final', r['id'].split('/')[1] + '.aux')
                    if os.path.isfile(p) and not same_file(p, os.path.join(dest, os.path.basename(p))):
                        shutil.copy(p, dest)
                if r['id'] == 'books/hott/hott-online' and r.get('dir'):
                    lines = []
                    for p in sorted(glob.glob(os.path.join(r['dir'], 'final', '*.aux'))):
                        with open(p, encoding='utf-8', errors='surrogateescape') as x:
                            lines += [ln for ln in x if ln.startswith('\\newlabel')]
                    write_if_changed(os.path.join(CORPUS, 'generated', 'hott', 'main.labels'), ''.join(lines))
    with open(os.path.join(HERE, 'sources.lock.json')) as f:
        lock = json.load(f)
    entries = []
    entries += stacks_entries(lock)
    entries += l3build_entries('latex2e', 'public-latex2e-tests')
    entries += l3build_entries('latex3', 'public-latex3-tests')
    for repo in ('hyperref', 'pgf', 'beamer', 'siunitx'):
        entries += l3build_entries(repo, 'public-package-tests')
    entries += document_entries()
    state_unused_roots(entries)
    timings = load_timings(args.timings)
    for e in entries:
        if e['id'] in DEEP_FAILURES:
            e['judge_failure'] = True
    assign_tiers(entries, timings)
    exclusions.extend(STATED_BOUNDS)
    pins = {k: {x: v.get(x) for x in ('repo', 'commit', 'tag', 'url', 'sha256', 'fetched')} for k, v in sorted(lock.items())}
    manifest = {'source': 'public',
                'description': 'Public LaTeX bodies and the official regression suites. Generated by tests/corpus/public/gen_manifest.py. '
                               'Tiers are not chosen by looking at results. In sha256(id) order per source: smoke is the first entries with a last pass under 2 s on a normal oracle '
                               f'({SMOKE_PER_SOURCE} and 1 per other source); standard is a fixed hash sample ({STANDARD_PERCENT}, else 40 percent) with a last pass under {TIME_LIMIT_MS // 1000} s; full is everything else. '
                               'Pinned sources: ' + json.dumps(pins, sort_keys=True),
                'defaults': {},
                'exclusions': exclusions,
                'entries': entries}
    with open(OUT, 'w') as f:
        json.dump(manifest, f, indent=1)
        f.write('\n')
    print(f'{len(entries)} entries, {len(exclusions)} exclusions -> {rel(OUT)}')
    return 0


if __name__ == '__main__':
    sys.exit(main())
