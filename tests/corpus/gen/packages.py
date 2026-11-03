"""Every LaTeX package, every class, every babel language: a load test, judged on log and PDF.

A package that does not load on the oracle stays in the set: the candidate has to fail the same way."""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))     # python3 -I leaves the script directory off the path
from common import TEXMF_DIST, clean_category, entry, write_doc

BODY = r"""Some text with math $\alpha+\beta=\int_0^1 f(x)\,dx$ and a display
\[ \sum_{i=1}^n i = \frac{n(n+1)}{2}. \]
Hyphenation and ligatures: office affluent difficult typesetting. \today
"""
END = r"""\begin{document}
""" + BODY + r"""\end{document}
"""
PACKAGE_DOC = '\\documentclass{article}\n\\usepackage{%s}\n' + END
CLASS_DOC = '\\documentclass{%s}\n' + END
BABEL_OPTION_DOC = '\\documentclass{article}\n\\usepackage[%s]{babel}\n' + END
BABEL_PROVIDE_DOC = '\\documentclass{article}\n\\usepackage[provide=*,%s]{babel}\n' + END
BABEL_INI_DOC = '\\documentclass{article}\n\\usepackage{babel}\n\\babelprovide[import=%s,main]{%s}\n' + END


def find_named(ext, roots):
    """{name: [paths]} for files with this extension under the roots."""
    res = {}
    for root in roots:
        for d, _, names in os.walk(root):
            for n in names:
                if n.endswith(ext):
                    res.setdefault(n[:-len(ext)], []).append(os.path.join(d, n))
    return res


def babel_lists():
    """(names with a .ldf that defines a language, names with a babel-NAME.tex locale stub, locale ids with a .ini,
    .ldf files that are not babel languages)."""
    ldf, not_lang = {}, []
    for name, paths in find_named('.ldf', [os.path.join(TEXMF_DIST, 'tex')]).items():
        for p in paths:
            with open(p, 'rb') as f:
                data = f.read()
            if b'\\ProvidesLanguage' in data or b'\\LdfInit' in data:
                ldf[name] = p
                break
        else:
            not_lang.append(name)
    locale = os.path.join(TEXMF_DIST, 'tex', 'generic', 'babel', 'locale')
    stubs = {n[len('babel-'):]: p[0] for n, p in find_named('.tex', [locale]).items() if n.startswith('babel-')}
    inis = {n[len('babel-'):]: p[0] for n, p in find_named('.ini', [locale]).items() if n.startswith('babel-')}
    return ldf, stubs, inis, sorted(not_lang)


def generate():
    entries = []
    roots = [os.path.join(TEXMF_DIST, 'tex', 'latex'), os.path.join(TEXMF_DIST, 'tex', 'generic')]
    stys = find_named('.sty', roots)
    clss = find_named('.cls', roots)
    for name in sorted(stys):
        main = write_doc('pkg', name, PACKAGE_DOC % name)
        entries.append(entry(f'coverage/pkg/{name}', main))
    for name in sorted(clss):
        main = write_doc('cls', name, CLASS_DOC % name)
        entries.append(entry(f'coverage/cls/{name}', main))
    ldf, stubs, inis, not_lang = babel_lists()
    options = sorted(set(ldf) | set(stubs))
    for name in options:
        main = write_doc('babel-option', name, BABEL_OPTION_DOC % name)
        entries.append(entry(f'coverage/babel-option/{name}', main))
    for name in sorted(stubs):
        main = write_doc('babel-provide', name, BABEL_PROVIDE_DOC % name)
        entries.append(entry(f'coverage/babel-provide/{name}', main))
    for name in sorted(inis):
        main = write_doc('babel-ini', name, BABEL_INI_DOC % (name, name))
        entries.append(entry(f'coverage/babel-ini/{name}', main))
    clean_category('pkg', stys)
    clean_category('cls', clss)
    clean_category('babel-option', options)
    clean_category('babel-provide', stubs)
    clean_category('babel-ini', inis)
    every_sty = find_named('.sty', [os.path.join(TEXMF_DIST, 'tex')])
    every_cls = find_named('.cls', [os.path.join(TEXMF_DIST, 'tex')])
    outside_sty = sorted(set(every_sty) - set(stys))
    outside_cls = sorted(set(every_cls) - set(clss))
    exclusions = [
        {'what': f'{len(outside_sty)} .sty names and {len(outside_cls)} .cls names found only outside tex/latex and tex/generic '
                 '(tex/latex-dev, tex/lualatex, tex/luatex, tex/xelatex, tex/xetex, tex/plain, tex/amstex)',
         'why': 'They belong to other engines or to the development kernel. rstex targets pdfTeX and the release kernel; the task named tex/latex and tex/generic.'},
        {'what': f'{len(not_lang)} .ldf files without \\ProvidesLanguage or \\LdfInit',
         'why': 'They are language helpers of other packages (datatool, datetime2, ...), not babel languages. Most are read by those packages when loaded.'},
    ]
    counts = {'pkg': len(stys), 'cls': len(clss), 'babel-option': len(options), 'babel-provide': len(stubs), 'babel-ini': len(inis)}
    return entries, exclusions, counts
