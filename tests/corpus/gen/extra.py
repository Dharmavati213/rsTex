#!/usr/bin/env -S python3 -I
r"""Tests for what tests/corpus/census.py found the rest of the corpus never reaches, or reaches only by luck.

    python3 -I tests/corpus/gen/extra.py          # writes target/corpus/gen/extra/ and tests/corpus/manifests/coverage-extra.json
    python3 -I tests/corpus/run.py prepare --manifest tests/corpus/manifests/coverage-extra.json -j 8

  extra/math/STYLE       Every math construct in each of the eight styles, all atom pairs included, as shipped boxes.  No trace prints a style, so the set of
                         styles reached by real documents cannot be measured.  This reaches all of them by construction, cramped ones through radicals.
  extra/hyph/en-NNN      \showhyphens over a word list.  A wrong hyphenation point shows in a document only when that word falls at a line end.
  extra/hyph-patterns/X  Every installed pattern file through \patterns in INITEX, then the same words.  What a format build does once, with every language.
  extra/state/X          \meaning of every name in a format and its packages' source, at \end{document}.  A wrong macro nothing uses is on no page and in no log otherwise.
  extra/format/X         The real format builds, judged on the log and on a .fmt existing.
  extra/pdf/X            PDF features nothing else in the corpus writes, by primitive.

Deterministic, like the other generators.  Judged on the log and, where there is one, the PDF.
"""
import json
import os
import re
import subprocess
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common  # noqa: E402

MANIFEST = os.path.join(common.REPO, 'tests', 'corpus', 'manifests', 'coverage-extra.json')
CAT = 'extra'
WORD_SOURCE = os.path.join(common.REPO, 'tests', 'ladder', '03-plain-texweb.tex')
RADICAL = r'\radical"270370'
STYLES = {'D': (r'$$', r'$$', ''), 'Dprime': (r'$$' + RADICAL + '{', '}$$', ''), 'T': ('$', '$', ''), 'Tprime': ('$' + RADICAL + '{', '}$', ''),
          'S': ('$x^{', '}$', ''), 'Sprime': ('$x^{' + RADICAL + '{', '}}$', ''), 'SS': ('$x^{y^{', '}}$', ''), 'SSprime': ('$x^{y^{' + RADICAL + '{', '}}}$', '')}
ATOMS = {'Ord': r'\mathord{a}', 'Op': r'\mathop{a}', 'Bin': r'\mathbin{+}', 'Rel': r'\mathrel{=}', 'Open': r'\mathopen{(}', 'Close': r'\mathclose{)}', 'Punct': r'\mathpunct{,}', 'Inner': r'\mathinner{a}'}
CONSTRUCTS = {
    'char': 'a', 'bin-at-start': '+a', 'bin-at-end': 'a+', 'rel': 'a=b', 'delims': '(a)', 'punct': 'a,b',
    'op-char': r'\mathop{\rm lim}', 'op-limits': r'\mathop{\rm lim}\limits_a^b', 'op-nolimits': r'\mathop{\rm lim}\nolimits_a^b', 'op-displaylimits': r'\mathop{\rm lim}\displaylimits_a^b',
    'sum': r'\sum_a^b', 'int': r'\int_a^b', 'over': r'{a\over b}', 'atop': r'{a\atop b}', 'above': r'{a\above 2pt b}', 'above-0pt': r'{a\above 0pt b}', 'overwithdelims': r'{a\overwithdelims() b}',
    'atopwithdelims': r'{a\atopwithdelims[] b}', 'abovewithdelims': r'{a\abovewithdelims\{\} 1pt b}', 'left-right': r'\left(a\over b\right)', 'left-middle': r'\left(a\middle|b\right)', 'left-null': r'\left.a\right|',
    'bigg-delim': r'\left(\vrule height 30pt depth 30pt\right)', 'underline': r'\underline a', 'overline': r'\overline a', 'accent': r'\hat a', 'accent-wide': r'\widehat{ab}', 'accent-char': r'\mathaccent"7014 a',
    'accent-script': r'\hat{a}^b', 'vcenter': r'\vcenter{\hbox{a}}', 'mathchoice': r'\mathchoice{d}{t}{s}{y}', 'radical': r'\sqrt{a}', 'radical-tall': r'\sqrt{\vrule height 20pt}', 'sup': 'a^b', 'sub': 'a_b', 'supsub': 'a^b_c',
    'sup-nested': 'a^{b^c}', 'sup-big': r'a^{\vrule height 12pt}', 'sub-big': r'a_{\vrule depth 12pt}', 'prime': "a'", 'mkern': r'a\mkern9mu b', 'mskip': r'a\mskip3mu plus 1mu minus 1mu b', 'mskip-fil': r'a\mskip 3mu plus 1fil b',
    'nonscript': r'a\nonscript\mskip9mu b', 'discretionary': r'a\discretionary{-}{}{}b', 'penalty': r'a\penalty7 b', 'rule': r'\vrule width 2pt height 2pt', 'hbox': r'\hbox{x}', 'inner-fraction': r'a{b\over c}d', 'mathsurround': r'\mathsurround 3pt a',
    'fam-rm': r'\rm ab', 'fam-it': r'\it fi', 'fam-bf': r'\bf ab', 'fam-tt': r'\tt ab', 'ligature-it': r'\it ff', 'kern-it': r'\it VA', 'italic-correction': r'\it f\/x', 'mark': r'a\mark{m}b', 'vadjust': r'a\vadjust{\kern1pt}b', 'insert': r'a\insert100{\kern1pt}b',
    'displaystyle': r'\displaystyle a', 'textstyle': r'\textstyle a', 'scriptstyle': r'\scriptstyle a', 'scriptscriptstyle': r'\scriptscriptstyle a', 'style-switch': r'a\scriptstyle b\displaystyle c', 'char-big': r'\mathchar"1350', 'delimiter': r'\delimiter"4266308 ', 'empty-group': '{}', 'empty-sup': '{}^a', 'empty-sub': '{}_a', 'kern-in-math': r'a\kern3pt b', 'glue-in-math': r'a\hskip3pt plus 1pt b',
    'eqno-free': r'a\ b', 'ampersand-free': r'a\mathsurround0pt',
}
HEADER = r"""\catcode`\@=11
\showboxbreadth=100000 \showboxdepth=100000 \tracingonline=0 \errorcontextlines=2
\scrollmode \batchmode
\newbox\unitbox
\def\unit#1#2{\setbox\unitbox=\hbox{#2}\immediate\write-1{[[unit #1]]}\showbox\unitbox \shipout\box\unitbox}
\def\dunit#1#2{\setbox\unitbox=\vbox{\hsize=300pt \noindent#2\par}\immediate\write-1{[[unit #1]]}\showbox\unitbox \shipout\box\unitbox}
\def\sqrtfree{}
"""


def math_doc(style):
    start, end, _ = STYLES[style]
    display = style.startswith('D')
    use = r'\dunit' if display else r'\unit'
    lines = [HEADER]
    units = []

    def one(name, body):
        lines.append(f'{use}{{{name}}}{{{start}{body}{end}}}')
        units.append(name)
    for name, body in CONSTRUCTS.items():
        one(name, body)
    for a, x in ATOMS.items():
        for b, y in ATOMS.items():
            one(f'pair-{a}-{b}', x + y)
    if display:                                       # what exists only in display math
        for name, body in {'eqno': r'$$a\eqno(1)$$', 'leqno': r'$$a\leqno(1)$$', 'eqno-long': r'$$\vrule width 280pt\eqno(1)$$', 'halign': r'$$\halign{#\cr a\cr}$$', 'display-after-text': r'x$$a$$y', 'short-skip': r'x$$\vcenter{\hbox{a}}$$',
                           'predisplay': r'\predisplaypenalty=7 \postdisplaypenalty=9 x$$a$$y', 'displayindent': r'\parshape 1 20pt 200pt x$$a$$y', 'hangindent': r'\hangindent=30pt \hangafter=-1 x$$a$$y', 'mathsurround-display': r'\mathsurround 5pt x$$a$$y'}.items():
            lines.append(rf'\dunit{{{name}}}{{{body}}}')
            units.append(name)
    lines.append(r'\bye')
    return '\n'.join(lines) + '\n', units


def word_list():
    with open(WORD_SOURCE, 'rb') as f:
        text = f.read().decode('latin-1')
    words = sorted({w for w in re.findall(r'\b[a-z]{5,}\b', text)})
    return words


def hyph_docs(words):
    """Plain TeX's \\showhyphens, which prints the word with its breaks in an Underfull line."""
    docs = {}
    chunk = 150
    per_doc = 40
    for d in range(0, len(words), chunk * per_doc):
        lines = [r'\batchmode']
        part = words[d:d + chunk * per_doc]
        for i in range(0, len(part), chunk):
            lines.append(r'\showhyphens{' + ' '.join(part[i:i + chunk]) + '}')
        lines.append(r'\bye')
        docs[f'en-{d // (chunk * per_doc):03d}'] = '\n'.join(lines) + '\n'
    return docs


def pattern_files():
    root = os.path.join(common.TEXMF_DIST, 'tex', 'generic', 'hyph-utf8', 'patterns', 'tex')
    try:
        return sorted(n for n in os.listdir(root) if n.startswith('hyph-') and n.endswith('.tex'))
    except OSError:
        return []


def pattern_doc(name, words):
    # In INITEX: the catcodes that hyph-*.tex and a font need, \patterns through that file, and then a very narrow paragraph, which breaks at every point the
    # patterns allow.  The box dump shows the discretionaries, and \tracingparagraphs what the line breaker saw.
    return (r'''\catcode`\{=1 \catcode`\}=2 \catcode`\^=7 \catcode`\#=6 \catcode`\^^I=10 \batchmode
\def\lettersfrom#1{\ifnum#1<123 \lccode#1=#1 \catcode#1=11 \lccode\numexpr#1-32\relax=#1 \catcode\numexpr#1-32\relax=11 \expandafter\lettersfrom\expandafter{\the\numexpr#1+1\expandafter}\fi}\lettersfrom{97}
\def\highfrom#1{\ifnum#1<256 \lccode#1=#1 \catcode#1=11 \expandafter\highfrom\expandafter{\the\numexpr#1+1\expandafter}\fi}\highfrom{128}
\font\f=cmr10 \f \hyphenchar\f=45 \lefthyphenmin=2 \righthyphenmin=2
\language=1 \input ''' + name + r'''
\showboxbreadth=100000 \showboxdepth=100000 \tracingonline=0 \hsize=1pt \parindent=0pt \parfillskip=0pt \pretolerance=-1 \tolerance=10000 \hbadness=10000 \hfuzz=16383pt \hyphenpenalty=-1000 \exhyphenpenalty=0 \emergencystretch=16383pt
\setbox0=\vbox{\noindent ''' + ' '.join(words) + r'''\par}
\showbox0
\pdfoutput=0 \end
''')


def run_names(doc_text, workdir):
    """Which files does this document read, and so which names can it have?"""
    with open(os.path.join(workdir, 'probe.tex'), 'w') as f:
        f.write(doc_text)
    env = dict(os.environ, **common.ORACLE_ENV)
    env['openin_any'] = 'a'
    subprocess.run(['pdflatex', '-interaction=batchmode', '-recorder', '-no-shell-escape', 'probe.tex'], cwd=workdir, env=env, capture_output=True, timeout=300)
    names = set()
    try:
        with open(os.path.join(workdir, 'probe.fls'), 'rb') as f:
            fls = f.read().decode('latin-1')
    except OSError:
        return names
    for line in fls.splitlines():
        if line.startswith('INPUT ') and re.search(r'\.(tex|sty|cls|clo|def|ldf|cfg|ltx|fd|ini)$', line) and '/probe.' not in line:
            names |= scan_names(line[6:])
    return names


def scan_names(path):
    try:
        with open(path, 'rb') as f:
            data = f.read().decode('latin-1')
    except OSError:
        return set()
    return {m for m in re.findall(r'\\([A-Za-z@_:]+)', data) if len(m) < 80}


STATE_SETS = {
    'latex-base': (r'\documentclass{article}', None),
    'amsmath': (r'\documentclass{article}\usepackage{amsmath,amssymb,amsthm,mathtools}', None),
    'hyperref': (r'\documentclass{article}\usepackage{hyperref}', None),
    'tikz': (r'\documentclass{article}\usepackage{tikz}', None),
    'tikz-cd': (r'\documentclass{article}\usepackage{tikz-cd}', None),
    'microtype': (r'\documentclass{article}\usepackage{microtype}', None),
    'babel': (r'\documentclass{article}\usepackage[german,english]{babel}', None),
    'fonts': (r'\documentclass{article}\usepackage[T1]{fontenc}\usepackage{lmodern,textcomp}', None),
    'layout': (r'\documentclass{article}\usepackage{geometry,xcolor,graphicx,enumitem,booktabs,longtable,caption,fancyhdr}', None),
    'expl3-xparse': (r'\documentclass{article}\usepackage{xparse,l3keys2e,siunitx}', None),
    'beamer': (r'\documentclass{beamer}', None),
    'memoir': (r'\documentclass{memoir}', None),
    'koma': (r'\documentclass{scrbook}', None),
    'amsart': (r'\documentclass{amsart}', None),
}
STATE_BODY = r"""
\begin{document}
\makeatletter
\newread\statefile
\openin\statefile=names.txt
\begingroup \endlinechar=-1 \catcode`\%=12
\def\stateloop{\readline\statefile to\stateline \ifeof\statefile\else
  \begingroup \expandafter\let\expandafter\stateval\csname\stateline\endcsname \immediate\write-1{\stateline=\meaning\stateval}\endgroup
  \expandafter\stateloop\fi}
\stateloop
\endgroup
x
\end{document}
"""
PDF_FEATURES = {
    # Written after census.py: each is a feature it found on no page of the corpus.  One document per feature, so a failure names it.
    'annot-text': r'\pdfannot width 10pt height 10pt depth 0pt {/Subtype /Text /Contents (note) /Name /Comment}\null',
    'annot-freetext': r'\pdfannot width 40pt height 10pt {/Subtype /FreeText /DA (/Helv 9 Tf 0 g) /Contents (free)}\null',
    'annot-stamp': r'\immediate\pdfobj{<< /Type /Border >>}\pdfannot width 40pt height 10pt {/Subtype /Stamp /Name /Draft}\null',
    'link-goto-page': r'\pdfstartlink user{/Subtype /Link /A << /S /GoTo /D [\pdfpageref 1\space 0 R /Fit] >>}here\pdfendlink',
    'link-gotor': r'\pdfstartlink attr{/Border [0 0 1]} goto file {other.pdf} page 3 {/Fit}here\pdfendlink',
    'link-gotor-name': r'\pdfstartlink goto file {other.pdf} name {dest} newwindow here\pdfendlink',
    'link-uri-attr': r'\pdfstartlink attr{/C [1 0 0]} user{/Subtype /Link /A << /S /URI /URI (http://a.example/) /IsMap true >>}here\pdfendlink',
    'link-launch': r'\pdfstartlink user{/Subtype /Link /A << /S /Launch /F (x.pdf) >>}here\pdfendlink',
    'link-named': r'\pdfstartlink user{/Subtype /Link /A << /S /Named /N /NextPage >>}here\pdfendlink',
    'link-javascript': r'\pdfstartlink user{/Subtype /Link /A << /S /JavaScript /JS (app.alert(1)) >>}here\pdfendlink',
    'link-thread': r'\pdfthread num 3 width 10pt height 10pt\null \pdfstartlink thread num 3 here\pdfendlink',
    'link-spanning-lines': r'\hsize=30pt \pdfstartlink user{/Subtype /Link /A << /S /URI /URI (u) >>}several words that must cross a line\pdfendlink',
    'link-running-off': r'\pdfstartlink user{/Subtype /Link /A << /S /URI /URI (u) >>}a\pdfrunninglinkoff b\pdfrunninglinkon c\pdfendlink',
    'thread-beads': r'\pdfstartthread num 7 attr{/Title (t)} text\par\pdfendthread\par\pdfthread name {seven} width 20pt height 20pt depth 1pt\null',
    'dest-forms': r'\pdfdest name{a} xyz\null\pdfdest name{b} fit\null\pdfdest name{c} fith\null\pdfdest name{d} fitv\null\pdfdest name{e} fitb\null\pdfdest name{f} fitbh\null\pdfdest name{g} fitbv\null\pdfdest name{h} fitr width 3pt height 3pt depth 1pt\null\pdfdest num 9 xyz zoom 2000\null',
    'outline-forms': r'\pdfoutline goto name{x} count 2 {Top}\pdfoutline goto page 1 {/Fit} {First}\pdfoutline goto num 9 count -1 {Second}\pdfoutline goto name{x} {Under second}\pdfoutline user{<< /S /URI /URI (u) >>} {Link}\pdfoutline attr{/C [1 0 0] /F 3} goto name{x} {Coloured}\pdfdest name{x} fit\pdfdest num 9 fit\null',
    'page-attrs': r'\pdfpageattr{/Rotate 90 /CropBox [0 0 300 300] /TrimBox [1 1 299 299] /BleedBox [0 0 300 300] /ArtBox [2 2 298 298] /Trans << /S /Wipe /D 1 >> /Dur 3 /UserUnit 2 /Tabs /S /LastModified (D:20000101)}\null',
    'pages-attrs': r'\pdfpagesattr{/Rotate 180}\null\vfill\eject\null',
    'catalog-all': r'\pdfcatalog{/PageLayout /TwoColumnLeft /PageMode /UseThumbs /Lang (en) /ViewerPreferences << /HideToolbar true >> /MarkInfo << /Marked false >> /PageLabels << /Nums [0 << /S /r >>] >> /OCProperties << /OCGs [] /D << /Order [] >> >>}\null',
    'catalog-openaction-goto': r'\pdfcatalog{} openaction goto page 1 {/Fit}\null',
    'catalog-openaction-user': r'\pdfcatalog{} openaction user{<< /S /Named /N /FirstPage >>}\null',
    'names-extra': r'\pdfnames{/JavaScript << /Names [(a) << /S /JavaScript /JS (x) >>] >>}\pdfdest name{a} fit\null',
    'info-all': r'\pdfinfo{/Title (t) /Author (a) /Subject (s) /Keywords (k) /Creator (c) /Producer (mine) /CreationDate (D:20000101) /ModDate (D:20000102) /Trapped /True /Custom (x)}\null',
    'trailer': r'\pdftrailer{/Custom 1}\null',
    'trailerid': r'\pdftrailerid{abc}\null',
    'infoomitdate': r'\pdfinfoomitdate=1 \null',
    'omitinfodict': r'\pdfomitinfodict=1 \null',
    'suppressptexinfo': r'\pdfsuppressptexinfo=15 \null',
    'minor-version-3': r'\pdfminorversion=3 \pdfobjcompresslevel=0 \null',
    'minor-version-7': r'\pdfminorversion=7 \null',
    'major-version-2': r'\pdfmajorversion=2 \pdfminorversion=0 \null',
    'compress-0': r'\pdfcompresslevel=0 \null',
    'objcompress-0': r'\pdfobjcompresslevel=0 \null',
    'objcompress-1': r'\pdfobjcompresslevel=1 \null',
    'objcompress-3': r'\pdfobjcompresslevel=3 \null',
    'decimaldigits-0': r'\pdfdecimaldigits=0 \vrule width 1.2345pt\kern1.2345pt x',
    'decimaldigits-4': r'\pdfdecimaldigits=4 \vrule width 1.23456pt\kern1.23456pt x',
    'xform-group-matrix': r'\setbox0=\hbox{x}\immediate\pdfxform attr{/Group << /S /Transparency /CS /DeviceCMYK >> /Matrix [1 0 0 1 3 3]} resources{/ExtGState << /G << /CA 0.5 >> >>} 0 \pdfrefxform\pdflastxform',
    'literal-forms': r'\pdfliteral{0 0 m 3 3 l S}\pdfliteral direct{q Q}\pdfliteral page{q Q}x\pdfliteral shipout{q Q}',
    'colorstack': r'\pdfcolorstackinit page direct{0 g}\edef\cs{\the\pdflastobj}\pdfcolorstack0 push{1 0 0 rg}x\pdfcolorstack0 set{0 1 0 rg}y\pdfcolorstack0 pop z\pdfcolorstack0 current',
    'setmatrix': r'\pdfsave\pdfsetmatrix{0.5 0 0 0.5}x\pdfrestore',
    'textmodes': r'\pdfliteral{1 Tr}x\pdfliteral{0 Tr}\pdfliteral{3 Ts}y\pdfliteral{0 Ts}\pdfliteral{50 Tz}z\pdfliteral{100 Tz}',
    'gstate-ops': r'\pdfliteral{q 2 w 1 J 1 j 3 M [2 1] 0 d /Perceptual ri 0.5 i 1 0 0 RG 0 1 0 rg 0.1 0.2 0.3 0.4 K 0.1 0.2 0.3 0.4 k 0.5 G 0.5 g 0 0 10 10 re B* 0 0 m 10 0 10 10 0 10 c 10 10 l y h b* W n Q}x',
    'marked-content': r'\pdfliteral{/Span << /Lang (en) >> BDC}x\pdfliteral{EMC /Artifact BMC}y\pdfliteral{EMC /Tag MP /Tag <<>> DP BX EX}',
    'inline-image': r'\pdfliteral{q 8 0 0 8 0 0 cm BI /W 1 /H 1 /BPC 8 /CS /G ID x EI Q}x',
    'pk-font': None,  # stays out: PK is dropped (DESIGN section 3) and mktexpk is off.  Named so the bound is stated.
    'font-no-embed': r'\pdfmapline{+cmss10 CMSS10}\font\x=cmss10 {\x Hello}',
    'font-whole-embed': r'\pdfmapline{+cmss10 CMSS10 <<cmss10.pfb}\font\x=cmss10 {\x Hello}',
    'font-slant': r'\pdfmapline{+cmss10 CMSS10 " .167 SlantFont " <cmss10.pfb}\font\x=cmss10 {\x Hello}',
    'font-extend': r'\pdfmapline{+cmss10 CMSS10 " .8 ExtendFont " <cmss10.pfb}\font\x=cmss10 {\x Hello}',
    'font-encoding': r'\pdfmapline{+ec-lmr10 LMRoman10-Regular " enclmEC ReEncodeFont " <lm-ec.enc <lmr10.pfb}\font\x=ec-lmr10 {\x Hello}',
    'font-omit-charset': r'\pdfomitcharset=1 \pdfmapline{+cmss10 CMSS10 <cmss10.pfb}\font\x=cmss10 {\x Hello}',
    'font-gentounicode': r'\pdfgentounicode=1 \pdfglyphtounicode{H}{0048}\pdfglyphtounicode{tfm:cmss10/e}{0065}\font\x=cmss10 {\x Hello}',
    'font-nobuiltin-tounicode': r'\pdfgentounicode=1 \pdfnobuiltintounicode\font \font\x=cmss10 \pdfnobuiltintounicode\x {\x Hello}',
    'font-includechars': r'\font\x=cmss10 \pdfincludechars\x{ABC}{\x Hello}',
    'font-attr': r'\font\x=cmss10 \pdffontattr\x{/Mine 1}{\x Hello}',
    'font-copy': r'\font\x=cmss10 \pdfcopyfont\y=\x {\y Hello}',
    'font-letterspace': r'\font\x=cmss10 \letterspacefont\y=\x 50 {\y Hello}',
    'font-expand-auto': r'\font\x=cmss10 \pdffontexpand\x 20 20 5 autoexpand \pdfadjustspacing=2 \hsize=100pt \x Hello there world, a paragraph long enough to break at least once or twice over its lines.\par',
    'font-protrude': r'\font\x=cmss10 \rpcode\x`.=200 \lpcode\x`H=100 \pdfprotrudechars=2 \hsize=100pt \x Hello there world. A paragraph long enough to break at least once or twice over its lines.\par',
    'font-interword': r'\font\x=cmss10 \pdfadjustinterwordglue=1 \knbscode\x`e=100 \stbscode\x`e=50 \shbscode\x`e=50 \pdfprependkern=1 \pdfappendkern=1 \knbccode\x`H=10 \knaccode\x`e=10 \x Hello there.\par',
    'font-noligatures': r'\font\x=cmss10 \pdfnoligatures\x {\x office}',
    'lookup-output-directory': r'\immediate\openout3=gen.tex \immediate\write3{\relax}\immediate\closeout3 \openin1=gen \ifeof1 \message{no gen}\else \message{gen found}\fi \closein1 \openin1=gen.tex \ifeof1 \message{no gen.tex}\else \message{gen.tex found}\fi \closein1 x',  # the output directory is searched for the name as given only: a quirk of open_input that a port must keep
    'origins': r'\pdfhorigin=1in \pdfvorigin=2in \pdfpagewidth=200pt \pdfpageheight=300pt \pdflinkmargin=2pt \pdfdestmargin=2pt \pdfthreadmargin=2pt x',
    'image-attrs': None,
}
PLAIN_PREFIX = '\\pdfoutput=1\n'


def main():
    entries = []
    # math
    for style in STYLES:
        text, units = math_doc(style)
        p = common.write_doc(CAT + '/math', style, text)
        entries.append({'id': f'extra/math/{style}', 'mode': 'plain', 'main': common.rel(p), 'units': units, 'tier': 'standard', 'gate_log': True})
    # hyphenation
    words = word_list()
    for name, text in hyph_docs(words).items():
        p = common.write_doc(CAT + '/hyph', name, text)
        entries.append({'id': f'extra/hyph/{name}', 'mode': 'plain', 'main': common.rel(p), 'tier': 'standard' if name.endswith('000') else 'full', 'gate_log': True, 'compare': 'log'})
    sample = words[:: max(1, len(words) // 2000)]
    for name in pattern_files():
        p = common.write_doc(CAT + '/hyph-patterns', name[:-4], pattern_doc(name, sample))
        entries.append({'id': f'extra/hyph-patterns/{name[:-4]}', 'mode': 'ini', 'first_line': '*doc.tex', 'jobname': 'doc', 'cwd': common.rel(os.path.dirname(p)), 'tier': 'full', 'compare': 'log', 'gate_log': True})
    # format state
    with tempfile.TemporaryDirectory(dir=common.GEN_ROOT) as tmp:
        for name, (head, _) in STATE_SETS.items():
            names = run_names(head + '\n\\begin{document}x\\end{document}\n', tmp)
            names |= scan_names(os.path.join(common.TEXMF_DIST, 'tex/latex/base/latex.ltx'))
            names |= scan_names(os.path.join(common.TEXMF_DIST, 'tex/latex/l3kernel/expl3-code.tex'))
            d = os.path.join(common.GEN_ROOT, CAT, 'state', name)
            p = common.write_doc(CAT + '/state', name, head + STATE_BODY)
            common.write_doc(CAT + '/state', name, ''.join(n + '\n' for n in sorted(names)), filename='names.txt')
            entries.append({'id': f'extra/state/{name}', 'main': common.rel(p), 'tier': 'standard' if name == 'latex-base' else 'full', 'gate_log': True, 'timeout': 120, 'notes': f'{len(names)} names'})
            for tail in ('probe.aux', 'probe.log', 'probe.fls', 'probe.pdf', 'probe.nav', 'probe.out', 'probe.snm', 'probe.toc'):
                try:
                    os.unlink(os.path.join(tmp, tail))
                except OSError:
                    pass
            _ = d
    # The real format builds: every document depends on them, and only a fixture did one before.  fmtutil.cnf has the arguments; a candidate's own .fmt is not compared.
    for name, prog, ini, language in (('pdflatex', 'pdflatex', '*pdflatex.ini', None), ('latex', 'latex', '*latex.ini', None), ('pdftex', 'pdftex', '*pdfetex.ini', None), ('etex', 'etex', '*etex.ini', None)):
        d = os.path.dirname(common.write_doc(CAT + '/format', name, '% This directory is where the format build runs.\n', filename='cwd.txt'))
        entries.append({'id': f'extra/format/{name}', 'mode': 'ini-format', 'first_line': ini, 'jobname': name, 'cwd': common.rel(d), 'args': [f'-progname={prog}', '-translate-file=cp227.tcx'],
                        'tier': 'standard', 'compare': 'log', 'gate_log': True, 'trusted': True, 'timeout': 120})
    # PDF features
    skipped = []
    for name, body in PDF_FEATURES.items():
        if body is None:
            skipped.append(name)
            continue
        lead = r'\leavevmode ' if name.startswith(('link-', 'font-', 'literal', 'colorstack', 'setmatrix', 'textmodes', 'gstate', 'marked', 'inline')) else ''
        p = common.write_doc(CAT + '/pdf', name, PLAIN_PREFIX + lead + body + '\n\\bye\n')
        entries.append({'id': f'extra/pdf/{name}', 'mode': 'plain', 'main': common.rel(p), 'tier': 'standard', 'gate_log': True, 'judge_failure': True})
    with open(MANIFEST, 'w') as f:
        json.dump({'source': 'coverage-extra', 'description': 'What tests/corpus/census.py found nothing else reaches. Written by tests/corpus/gen/extra.py; the documents are under target/corpus/gen/extra.',
                   'exclusions': [{'what': 'A PDF feature list that is complete', 'why': f'extra/pdf/ names the features census.py did not see. Left out on purpose: {", ".join(skipped)}: PK fonts are dropped and mktexpk is off; image attributes are covered by coverage/prim-image.'},
                                  {'what': 'Hyphenation patterns of languages not installed', 'why': 'This TeX Live has texlive-langenglish and texlive-langgerman only, so extra/hyph-patterns has the files that exist.'}],
                   'entries': entries}, f, indent=1)
    print(f'{len(entries)} entries: ' + ', '.join(f'{k} {sum(e["id"].startswith("extra/" + k + "/") for e in entries)}' for k in ('math', 'hyph', 'hyph-patterns', 'state', 'format', 'pdf')))
    print(f'units in math: {sum(len(e.get("units", [])) for e in entries)}; words in hyph: {len(words)}')
    return 0


if __name__ == '__main__':
    sys.exit(main())
