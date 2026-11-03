r"""One document per primitive, in plain mode, judged on the log and the PDF.

coverage/prim/NAME       what the primitive reports about itself, its value edges if it is a parameter, the standard scenario under
                         a few values of the parameter, and the hand-written probes of probes_core.py and probes_ext.py.
coverage/prim-bare/NAME  the primitive used bare in vertical, horizontal and math mode.  Kept apart because a bare use can end the job.
coverage/prim-fatal/...  paths that end the job: the run has no PDF and the log and exit status are what is judged.
coverage/prim-ini/...    INITEX: compatibility mode, patterns, \dump.

A document carries \T{..} lines, which reach only the log.  [[end of probes]] is the last one, and reach.py uses it to tell a document
that ran to its end from one that stopped early.
"""
import concurrent.futures as cf
import hashlib
import json
import os
import re
import shutil
import struct
import subprocess
import tempfile
import zlib
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))     # python3 -I leaves the script directory off the path
import probes_core
import probes_ext  # noqa: F401  (extends probes_core.SPECIFIC)
from common import GEN_ROOT, REPO, clean_category, entry, run_pdftex, write_doc, write_json

SPECIFIC = probes_core.SPECIFIC
TRIAL_TIMEOUT = 15     # seconds for one trial compile while refining a probe
PLAN_VERSION = 2       # bump when blocks_of or refine changes, since the plan is cached by the text of the probe

INT = """pretolerance tolerance linepenalty hyphenpenalty exhyphenpenalty clubpenalty widowpenalty displaywidowpenalty brokenpenalty
binoppenalty relpenalty predisplaypenalty postdisplaypenalty interlinepenalty doublehyphendemerits finalhyphendemerits adjdemerits mag
delimiterfactor looseness time day month year showboxbreadth showboxdepth hbadness vbadness pausing tracingonline tracingmacros tracingstats
tracingparagraphs tracingpages tracingoutput tracinglostchars tracingcommands tracingrestores uchyph outputpenalty maxdeadcycles hangafter
floatingpenalty globaldefs fam escapechar defaulthyphenchar defaultskewchar endlinechar newlinechar language lefthyphenmin righthyphenmin
holdinginserts errorcontextlines tracingstacklevels partokencontext showstream pdfoutput pdfcompresslevel pdfobjcompresslevel
pdfdecimaldigits pdfmovechars pdfimageresolution pdfpkresolution pdfuniqueresname pdfoptionpdfminorversion pdfoptionalwaysusepdfpagebox
pdfoptionpdfinclusionerrorlevel pdfmajorversion pdfminorversion pdfforcepagebox pdfpagebox pdfinclusionerrorlevel pdfgamma pdfimagegamma
pdfimagehicolor pdfimageapplygamma pdfadjustspacing pdfprotrudechars pdftracingfonts pdfadjustinterwordglue pdfprependkern pdfappendkern
pdfgentounicode pdfdraftmode pdfinclusioncopyfonts pdfsuppresswarningdupdest pdfsuppresswarningdupmap pdfsuppresswarningpagegroup
pdfinfoomitdate pdfsuppressptexinfo pdfomitcharset pdfomitinfodict pdfomitprocset pdfptexuseunderscore tracingassigns tracinggroups tracingifs
tracingscantokens tracingnesting predisplaydirection lastlinefit savingvdiscards savinghyphcodes ignoreprimitiveerror TeXXeTstate synctex""".split()
DIMEN = """parindent mathsurround lineskiplimit hsize vsize maxdepth splitmaxdepth boxmaxdepth hfuzz vfuzz delimitershortfall nulldelimiterspace
scriptspace predisplaysize displaywidth displayindent overfullrule hangindent hoffset voffset emergencystretch pdfhorigin pdfvorigin
pdfpagewidth pdfpageheight pdflinkmargin pdfdestmargin pdfthreadmargin pdffirstlineheight pdflastlinedepth pdfeachlineheight pdfeachlinedepth
pdfignoreddimen pdfpxdimen""".split()
GLUE = """lineskip baselineskip parskip abovedisplayskip belowdisplayskip abovedisplayshortskip belowdisplayshortskip leftskip rightskip topskip
splittopskip tabskip spaceskip xspaceskip parfillskip""".split()
MUGLUE = 'thinmuskip medmuskip thickmuskip'.split()
TOKS = 'output everypar everymath everydisplay everyhbox everyvbox everyjob everycr errhelp pdfpagesattr pdfpageattr pdfpageresources pdfpkmode everyeof'.split()
PAGE_DIMEN = 'pagegoal pagetotal pagestretch pagefilstretch pagefillstretch pagefilllstretch pageshrink pagedepth'.split()
# Encoding-dependent extensions that exist only with -enc or -mltex; DESIGN section 3 drops them, and they must stay undefined here.
ABSENT = 'charsubdef charsubdefmax charsubdefmin endmubyte mubyte mubytein mubytelog mubyteout noconvert specialout tracingcharsubdef xchrcode xordcode xprncode'.split()

# Values to run the scenario under.  Default for a parameter not named here: DEFAULT_*.
DEFAULT_INT = ['100', '-100']
INT_VALUES = {
    'pretolerance': ['-1', '0', '10000'], 'tolerance': ['0', '200', '10000'], 'looseness': ['1', '-1'], 'hangafter': ['2', '-2'],
    'language': ['1', '255', '256'], 'lefthyphenmin': ['1', '5'], 'righthyphenmin': ['1', '5'], 'uchyph': ['0', '1'],
    'mag': ['2000', '0', '32769'], 'maxdeadcycles': ['0', '1'], 'holdinginserts': ['1', '0'], 'showboxbreadth': ['-1', '0', '5'],
    'showboxdepth': ['-1', '0', '3'], 'errorcontextlines': ['-1', '0', '100'], 'tracingonline': ['0', '1'], 'tracingmacros': ['0', '2'],
    'tracingstats': ['0', '1', '2'], 'tracingparagraphs': ['0', '1'], 'tracingpages': ['0', '1'], 'tracingoutput': ['0', '1'],
    'tracinglostchars': ['0', '1', '2', '3'], 'tracingcommands': ['0', '1', '2', '3'], 'tracingrestores': ['0', '1'], 'tracingassigns': ['0', '1'],
    'tracinggroups': ['0', '1'], 'tracingifs': ['0', '1'], 'tracingscantokens': ['0', '1'], 'tracingnesting': ['0', '1', '2'],
    'tracingstacklevels': ['1', '3'], 'globaldefs': ['1', '-1'], 'fam': ['-1', '1', '16'], 'escapechar': ['-1', '65'],
    'defaulthyphenchar': ['-1', '127'], 'defaultskewchar': ['-1', '127'], 'endlinechar': ['-1', '32'], 'newlinechar': ['10', '97'],
    'partokencontext': ['0', '1', '2'], 'showstream': ['-1', '5'], 'pdfoutput': ['0', '1'], 'pdfcompresslevel': ['0', '9'],
    'pdfobjcompresslevel': ['0', '1', '2', '3'], 'pdfdecimaldigits': ['0', '2', '4'], 'pdfmajorversion': ['1', '2'], 'pdfminorversion': ['4', '5', '7'],
    'pdfoptionpdfminorversion': ['4', '7'], 'pdfadjustspacing': ['0', '1', '2', '3'], 'pdfprotrudechars': ['0', '1', '2', '3'], 'pdftracingfonts': ['0', '1', '2', '3'],
    'pdfadjustinterwordglue': ['0', '1'], 'pdfprependkern': ['0', '1'], 'pdfappendkern': ['0', '1'], 'pdfgentounicode': ['0', '1'], 'pdfdraftmode': ['1'],
    'pdfinfoomitdate': ['1'], 'pdfsuppressptexinfo': ['-1', '1', '15'], 'pdfomitcharset': ['1'], 'pdfomitinfodict': ['1'], 'pdfomitprocset': ['1', '-1'],
    'pdfptexuseunderscore': ['1'], 'pdfuniqueresname': ['1'], 'predisplaydirection': ['1', '-1'], 'lastlinefit': ['1000', '500'],
    'savingvdiscards': ['1'], 'savinghyphcodes': ['1'], 'ignoreprimitiveerror': ['1'], 'TeXXeTstate': ['1'], 'synctex': ['0'],
    'pdfimageresolution': ['300'], 'pdfpkresolution': ['600'], 'pdfinclusioncopyfonts': ['1'], 'pdfinclusionerrorlevel': ['1', '-1'],
    'pdfpagebox': ['1', '5'], 'pdfforcepagebox': ['1'], 'pdfoptionalwaysusepdfpagebox': ['1'], 'pdfoptionpdfinclusionerrorlevel': ['1'],
    'pdfgamma': ['2200'], 'pdfimagegamma': ['2200'], 'pdfimagehicolor': ['0', '1'], 'pdfimageapplygamma': ['1'], 'pdfmovechars': ['1', '2'],
    'pdfsuppresswarningdupdest': ['1'], 'pdfsuppresswarningdupmap': ['1'], 'pdfsuppresswarningpagegroup': ['1'], 'pausing': ['1'],
    'hbadness': ['0', '10000'], 'vbadness': ['0', '10000'], 'time': ['0'], 'day': ['0'], 'month': ['0'], 'year': ['0'], 'outputpenalty': ['5'],
}
DEFAULT_DIMEN = ['1pt', '20pt', '-5pt']
DIMEN_VALUES = {
    'hsize': ['100pt', '1000pt', '10pt'], 'vsize': ['60pt', '400pt', '0pt'], 'maxdepth': ['0pt', '20pt'], 'hfuzz': ['0pt', '20pt'], 'vfuzz': ['0pt', '20pt'],
    'hangindent': ['10pt', '-10pt'], 'emergencystretch': ['10pt', '100pt'], 'overfullrule': ['0pt', '5pt'], 'pdfpagewidth': ['300pt', '0pt'], 'pdfpageheight': ['300pt', '0pt'],
    'pdfhorigin': ['0pt', '2in'], 'pdfvorigin': ['0pt', '2in'], 'hoffset': ['1in', '-1in'], 'voffset': ['1in', '-1in'], 'pdfpxdimen': ['1bp', '2pt'],
    'pdffirstlineheight': ['20pt', '0pt'], 'pdflastlinedepth': ['20pt', '0pt'], 'pdfeachlineheight': ['20pt', '0pt'], 'pdfeachlinedepth': ['20pt', '0pt'], 'pdfignoreddimen': ['0pt', '-1000pt'],
    'predisplaysize': ['10pt'], 'displaywidth': ['10pt'], 'displayindent': ['10pt'], 'pdflinkmargin': ['2pt'], 'pdfdestmargin': ['2pt'], 'pdfthreadmargin': ['2pt'],
}
DEFAULT_GLUE = ['3pt plus 1pt minus 1pt', '0pt plus 1fil']
GLUE_VALUES = {
    'baselineskip': ['20pt', '0pt plus 1fil', '-5pt'], 'lineskip': ['5pt plus 1pt'], 'parfillskip': ['0pt', '0pt plus 1fil minus 1pt', '5pt plus 1fill'],
    'leftskip': ['10pt', '0pt plus 1fil'], 'rightskip': ['10pt', '0pt plus 1fil'], 'spaceskip': ['5pt plus 1pt minus 1pt', '0pt'], 'xspaceskip': ['5pt plus 1pt minus 1pt', '0pt'],
    'topskip': ['50pt', '0pt plus 1fil'], 'splittopskip': ['20pt', '0pt plus 1fil'],
}
DEFAULT_MU = ['5mu', '1mu plus 1fil minus 1mu']
TOKS_VALUES = {
    'output': r'{\T{ran output}\shipout\box255 }', 'everycr': r'{\noalign{\T{ran everycr}}}', 'pdfpagesattr': r'{/Rotate 90}',
    'pdfpageattr': r'{/Rotate 90}', 'pdfpageresources': r'{/ProcSet [/PDF]}', 'pdfpkmode': r'{ljfour}', 'errhelp': r'{a help message}',
}

PRELUDE = r"""% generated by tests/corpus/gen/primitives.py
\def\T#1{\immediate\write-1{[[#1]]}}
\def\V#1{\immediate\write-1{#1}}
\begingroup\catcode`\%=12 \xdef\pc{%}\endgroup
\begingroup\catcode`\<=1 \catcode`\>=2 \catcode`\{=12 \catcode`\}=12 \xdef\lb<{>\xdef\rb<}>\endgroup
\hsize=200pt \vsize=120pt \parindent=10pt \tolerance=500 \hbadness=1000 \vbadness=1000 \hfuzz=0.1pt
\baselineskip=12pt \lineskip=1pt \lineskiplimit=0pt \parskip=0pt plus 1pt \topskip=10pt \maxdepth=2pt
\skip100=6pt plus 1pt \count100=1000 \dimen100=50pt
\output={\T{output penalty=\the\outputpenalty\space goal=\the\pagegoal\space total=\the\pagetotal\space insertpen=\the\insertpenalties
  \space marks=[\topmark][\firstmark][\botmark]}%
  \shipout\vbox{\unvbox255 \ifvoid100 \else\kern3pt\box100 \fi}\advance\count0 by 1 }
\def\scenariopar{Hyphenation of long words is an interesting phenomenon in typesetting systems;
  the paragraph builder considers feasible breakpoints under tolerance constraints, minimizing demerits and
  considering discretionaries such as ex\-am\-ple, as well as \hbox{unbreakable} boxes, kerns such as AVA,
  ligatures fi fl ff ffi ffl, accents \accent127 e \accent19 o and italic corrections {\it f\/}.\par}
\def\scenario{\T{scenario start}\showlists
  \scenariopar\mark{M1}\marks3{N3}%
  \noindent Some display math with a label
  $$\int_0^1 {x^2+y_1\over 1-z}\,dx = \sqrt{2}\,\left(a\over b\right) \eqno(1)$$
  and inline math $a+b=c$ and $f(x)\le g(x)\mathsurround=1pt$ with a mark\mark{M2}\insert100{\hbox{note}} and a
  vadjust\vadjust{\kern2pt} end of paragraph.\par
  \halign{\hfil##\tabskip=2pt plus 1pt&##\hfil\cr a&b\cr\noalign{\hrule}c&d\cr}%
  \penalty-50 \hrule height 1pt \vskip 3pt plus 1pt minus 1pt
  \setbox1=\vbox{\scenariopar}\showbox1
  \setbox2=\vsplit1 to 24pt \showbox2 \showbox1 \box2 \box1
  \hbox to 100pt{a\hfil b\leaders\hrule\hskip 10pt c}%
  \hbox{\vrule width 1pt \special{cov}}%
  \vfill\penalty-20000 \T{scenario end}}
\showboxbreadth=300 \showboxdepth=30 \errorcontextlines=4
\tracingonline=1 \tracingcommands=2 \tracingmacros=1 \tracingrestores=1 \tracinglostchars=1
\tracingoutput=1 \tracingpages=1 \tracingparagraphs=1 \tracingassigns=1 \tracinggroups=1 \tracingifs=1
\tracingscantokens=1 \tracingnesting=2
"""

BARE_PRELUDE = r"""% generated by tests/corpus/gen/primitives.py
\def\T#1{\immediate\write-1{[[#1]]}}
\showboxbreadth=300 \showboxdepth=30 \errorcontextlines=4
\tracingonline=1 \tracingcommands=2 \tracingrestores=1 \tracinglostchars=1 \tracinggroups=1 \tracingifs=1
\pdfsetrandomseed 1
"""

# A bare use would swallow the rest of the document for these: conditionals need their \fi, definitions eat the next token and
# break \relax, and these others end the job, which prim-fatal covers.  They have probes in the main document.
NO_BARE = set("""end dump endinput input read readline errorstopmode if ifcat ifnum ifdim ifodd ifvmode ifhmode ifmmode ifinner ifvoid ifhbox ifvbox
ifx ifeof iftrue iffalse ifcase ifpdfprimitive ifdefined ifcsname iffontchar ifincsname ifpdfabsnum ifpdfabsdim def gdef edef xdef let futurelet
long outer global protected chardef mathchardef countdef dimendef skipdef muskipdef toksdef font pdfcopyfont letterspacefont partokenname""".split())


def csname(p):
    return '\\' + p


def generic_part(p):
    cs = csname(p)
    return rf"""
\T{{generic}}
\show{cs}
\V{{\meaning{cs}}}
\V{{\string{cs}}}
\V{{\detokenize{{{cs}}}}}
\expandafter\show\csname {p}\endcsname
\let\x={cs}\show\x \V{{\meaning\x}}
\ifx\x{cs}\V{{let is same}}\else\V{{let differs}}\fi
\ifpdfprimitive{cs}\V{{is primitive}}\else\V{{is not primitive}}\fi
\ifpdfprimitive\x\V{{let is primitive}}\else\V{{let is not primitive}}\fi
\ifdefined{cs}\V{{is defined}}\else\V{{is undefined}}\fi
\ifcsname {p}\endcsname\V{{csname is defined}}\else\V{{csname is undefined}}\fi
\edef\y{{\noexpand{cs}}}\show\y
\begingroup\escapechar=-1 \V{{\string{cs}}}\escapechar=`! \V{{\string{cs}\meaning{cs}}}\endgroup
\begingroup\def{cs}{{redefined}}\show{cs}\V{{\meaning{cs}}}\ifpdfprimitive{cs}\V{{still primitive}}\else\V{{no longer primitive}}\fi\endgroup
\ifpdfprimitive{cs}\V{{primitive again}}\else\V{{not primitive again}}\fi
"""


def int_part(p):
    cs = csname(p)
    reset = r'\global\globaldefs=0 ' if p == 'globaldefs' else ''
    out = [rf"""
\T{{integer parameter}}
\showthe{cs}\V{{\the{cs},\number{cs}}}
\T{{integer edges}}
\begingroup
{cs}=1 \showthe{cs}{cs}=-1 \showthe{cs}{cs}=0 \showthe{cs}
{cs}=2147483647 \showthe{cs}{cs}=-2147483647 \showthe{cs}
{cs}=2147483648 \showthe{cs}
{cs}=-2147483648 \showthe{cs}
{cs}="7FFFFFFF \showthe{cs}{cs}='17 \showthe{cs}{cs}=`a \showthe{cs}{cs}=`\a \showthe{cs}
{cs}=+-+5 \showthe{cs}{cs}=--5 \showthe{cs}
{cs}=\baselineskip \showthe{cs}{cs}=\count0 \showthe{cs}{cs}={cs}\showthe{cs}
\endgroup{reset}
\begingroup
\setbox0=\hbox{{{cs}=x}}\setbox0=\hbox{{{cs}=\relax}}\showthe{cs}
\endgroup{reset}
\T{{integer arithmetic}}
\begingroup
{cs}=1000 \advance{cs} by 7 \showthe{cs}\multiply{cs} by 3 \showthe{cs}\divide{cs} by 2 \showthe{cs}
\divide{cs} by 0 \showthe{cs}\multiply{cs} by 2147483647 \showthe{cs}{cs}=2147483647 \advance{cs} by 1 \showthe{cs}
\endgroup{reset}
\begingroup\global{cs}={cs}\endgroup{reset}
\showthe{cs}
"""]
    for v in INT_VALUES.get(p, DEFAULT_INT):
        out.append(f'\\T{{scenario with {p}={v}}}\\begingroup{cs}={v} \\scenario\\endgroup{reset}\n')
    return ''.join(out)


def dimen_part(p):
    cs = csname(p)
    out = [rf"""
\T{{dimension parameter}}
\showthe{cs}\V{{\the{cs}}}
\T{{dimension edges}}
\begingroup
{cs}=1pt \showthe{cs}{cs}=0pt \showthe{cs}{cs}=-1sp \showthe{cs}
{cs}=16383.99999pt \showthe{cs}{cs}=-16383.99999pt \showthe{cs}
{cs}=16384pt \showthe{cs}
{cs}=1in \showthe{cs}{cs}=1cm \showthe{cs}{cs}=1mm \showthe{cs}{cs}=1bp \showthe{cs}{cs}=1pc \showthe{cs}
{cs}=1dd \showthe{cs}{cs}=1cc \showthe{cs}{cs}=1nd \showthe{cs}{cs}=1nc \showthe{cs}{cs}=1sp \showthe{cs}
{cs}=1px \showthe{cs}{cs}=1em \showthe{cs}{cs}=1ex \showthe{cs}{cs}=1truein \showthe{cs}
{cs}=\baselineskip \showthe{cs}{cs}=\count0 sp \showthe{cs}{cs}=1.5{cs}\showthe{cs}{cs}={cs}\showthe{cs}
\endgroup
\begingroup
\setbox0=\hbox{{{cs}=x}}\setbox0=\hbox{{{cs}=1}}\setbox0=\hbox{{{cs}=1mu}}\setbox0=\hbox{{{cs}=1xy}}\showthe{cs}
\endgroup
\T{{dimension arithmetic}}
\begingroup
{cs}=10pt \advance{cs} by 1pt \showthe{cs}\multiply{cs} by 3 \showthe{cs}\divide{cs} by 2 \showthe{cs}
\divide{cs} by 0 \showthe{cs}\multiply{cs} by 2147483647 \showthe{cs}{cs}=16383pt \advance{cs} by 16383pt \showthe{cs}
\endgroup
\begingroup\global{cs}={cs}\endgroup
\showthe{cs}
"""]
    for v in DIMEN_VALUES.get(p, DEFAULT_DIMEN):
        out.append(f'\\T{{scenario with {p}={v}}}\\begingroup{cs}={v} \\scenario\\endgroup\n')
    return ''.join(out)


def glue_part(p):
    cs = csname(p)
    out = [rf"""
\T{{glue parameter}}
\showthe{cs}\V{{\the{cs}}}
\T{{glue edges}}
\begingroup
{cs}=1pt \showthe{cs}{cs}=1pt plus 2pt \showthe{cs}{cs}=1pt minus 2pt \showthe{cs}{cs}=1pt plus 2pt minus 3pt \showthe{cs}
{cs}=1pt plus 1fil \showthe{cs}{cs}=1pt plus 1fill \showthe{cs}{cs}=1pt plus 1filll \showthe{cs}
{cs}=1pt minus 1fil \showthe{cs}{cs}=1pt minus 1fill \showthe{cs}{cs}=1pt minus 1filll \showthe{cs}
{cs}=16383.99999pt plus 16383.99999pt minus 16383.99999pt \showthe{cs}
{cs}=-1pt plus -1pt minus -1pt \showthe{cs}
{cs}=\parindent \showthe{cs}{cs}=\count0 sp \showthe{cs}{cs}={cs}\showthe{cs}{cs}=2{cs}\showthe{cs}
\endgroup
\begingroup
\setbox0=\hbox{{{cs}=16384pt}}\setbox0=\hbox{{{cs}=x}}\setbox0=\hbox{{{cs}=1pt plus x}}\setbox0=\hbox{{{cs}=1pt plus 1fillll}}\setbox0=\hbox{{{cs}=\thinmuskip}}\showthe{cs}
\endgroup
\T{{glue arithmetic}}
\begingroup
{cs}=10pt plus 1pt minus 2pt \advance{cs} by 1pt plus 1fil \showthe{cs}\multiply{cs} by 3 \showthe{cs}\divide{cs} by 2 \showthe{cs}
\divide{cs} by 0 \showthe{cs}\multiply{cs} by 2147483647 \showthe{cs}
\endgroup
\begingroup\global{cs}={cs}\endgroup
\showthe{cs}
"""]
    for v in GLUE_VALUES.get(p, DEFAULT_GLUE):
        out.append(f'\\T{{scenario with {p}={v}}}\\begingroup{cs}={v} \\scenario\\endgroup\n')
    return ''.join(out)


def mu_part(p):
    cs = csname(p)
    out = [rf"""
\T{{math glue parameter}}
\showthe{cs}\V{{\the{cs}}}
\begingroup
{cs}=1mu \showthe{cs}{cs}=1mu plus 2mu minus 3mu \showthe{cs}{cs}=1mu plus 1fil minus 1fill \showthe{cs}{cs}={cs}\showthe{cs}
{cs}=16383.99999mu \showthe{cs}
\setbox0=\hbox{{{cs}=1pt}}\setbox0=\hbox{{{cs}=x}}\setbox0=\hbox{{{cs}=16384mu}}\setbox0=\hbox{{{cs}=\baselineskip}}\showthe{cs}
{cs}=1mu \advance{cs} by 1mu plus 1fil \showthe{cs}\multiply{cs} by 3 \showthe{cs}\divide{cs} by 2 \showthe{cs}\divide{cs} by 0
\endgroup
\showthe{cs}
"""]
    for v in DEFAULT_MU:
        out.append(f'\\T{{scenario with {p}={v}}}\\begingroup{cs}={v} \\scenario\\endgroup\n')
    return ''.join(out)


def toks_part(p):
    cs = csname(p)
    value = TOKS_VALUES.get(p, r'{\T{ran %s}}' % p)
    return rf"""
\T{{token list parameter}}
\showthe{cs}\V{{\the{cs}}}
\begingroup
{cs}={{a#b\c}}\showthe{cs}{cs}={{}}\showthe{cs}{cs}=\toks0 \showthe{cs}{cs}={cs}\showthe{cs}
{cs}\expandafter{{\the{cs} x}}\showthe{cs}\edef\z{{\the{cs}}}\show\z
\toks0=\expandafter{{\the{cs}}}\showthe\toks0
\setbox0=\hbox{{{cs}=x}}\setbox0=\hbox{{{cs}=\relax}}\showthe{cs}
\endgroup
\begingroup\global{cs}={cs}\endgroup
\showthe{cs}
\T{{scenario with {p} set}}
\begingroup{cs}={value}\scenario\endgroup
"""


def page_dimen_part(p):
    cs = csname(p)
    return rf"""
\T{{page dimension}}
\showthe{cs}\V{{\the{cs}}}
\hbox{{a}}\showthe{cs}\penalty0 \hbox{{b}}\vskip 3pt plus 1pt plus 1fil minus 1pt\hbox{{c}}\showthe{cs}\showlists
{cs}=100pt \showthe{cs}\showlists
\advance{cs} by 1pt \showthe{cs}\multiply{cs} by 2 \showthe{cs}\divide{cs} by 2 \showthe{cs}
\begingroup{cs}=7pt \endgroup\showthe{cs}
\setbox0=\hbox{{\showthe{cs}{cs}=9pt \showthe{cs}}}\showthe{cs}
\setbox0=\hbox{{$\showthe{cs}$}}
{cs}=16384pt
{cs}=x
\par
\vfill\penalty-10000 \showthe{cs}
{cs}=5pt \showthe{cs}
\hbox{{d}}\showlists\showthe{cs}\vfill\penalty-10000
\pagegoal=\vsize
"""


def absent_doc(p):
    cs = csname(p)
    return PRELUDE + rf"""
\T{{{p} must be undefined without -enc and -mltex}}
\ifdefined{cs}\V{{defined}}\else\V{{undefined}}\fi
\ifcsname {p}\endcsname\V{{csname defined}}\else\V{{csname undefined}}\fi
\show{cs}
\V{{\meaning{cs}}}
{cs}
\T{{end of probes}}
\end
"""


def value_doc(p, v):
    """A setting that must hold from the start of the job and cannot change afterwards: one document per value."""
    head = PRELUDE + '\n\\T{%s=%s for the whole document}\n\\%s=%s \\showthe\\%s\n\\scenario\n' % (p, v, p, v, p)
    if p == 'pdfoutput' and v == '0':
        body = DVI_BODY
    else:
        body = '\\pdfinfo{/Title (value probe)}\n\\immediate\\pdfobj {<</Probe 1>>}\n\\scenario\n'
    return head + body + '\\T{end of probes}\n\\end\n'


def main_body(p):
    parts = [generic_part(p)]
    if p in INT:
        parts.append(int_part(p))
    elif p in DIMEN:
        parts.append(dimen_part(p))
    elif p in GLUE:
        parts.append(glue_part(p))
    elif p in MUGLUE:
        parts.append(mu_part(p))
    elif p in TOKS:
        parts.append(toks_part(p))
    elif p in PAGE_DIMEN:
        parts.append(page_dimen_part(p))
    if p in SPECIFIC:
        parts.append(SPECIFIC[p])
    return ''.join(parts)


def bare_doc(p):
    cs = csname(p)
    return BARE_PRELUDE + rf"""
\T{{bare in vertical mode}}
{cs} \relax
\showlists
\T{{bare in horizontal mode}}
\setbox0=\hbox{{{cs} \relax}}\showbox0
\T{{bare in math mode}}
\setbox0=\hbox{{${cs} \relax$}}\showbox0
\T{{bare in paragraph}}
\noindent x{cs} \relax y\par
\T{{bare in display}}
$${cs} \relax$$
\T{{bare in alignment}}
\setbox0=\vbox{{\halign{{#\cr{cs} \relax\cr}}}}\showbox0
\T{{end of probes}}
\end
"""


EXTRA_SPECIFIC = {
    'deadcycles': r"""
\T{deadcycles}
\showthe\deadcycles
\deadcycles=5 \showthe\deadcycles\deadcycles=0
\deadcycles=-1 \showthe\deadcycles\deadcycles=0
\output={\showthe\deadcycles\setbox0=\box255 }
\hbox{}\penalty-10000 \showthe\deadcycles\hbox{}\penalty-10000 \showthe\deadcycles\hbox{}\penalty-10000
\deadcycles=0
\output={\shipout\box255 \showthe\deadcycles}\hbox{}\penalty-10000 \showthe\deadcycles
\begingroup\deadcycles=7 \endgroup\showthe\deadcycles\deadcycles=0
\setbox0=\hbox{\showthe\deadcycles\deadcycles=1 }\showthe\deadcycles\deadcycles=0
\advance\deadcycles by 1 \showthe\deadcycles\multiply\deadcycles by 5 \showthe\deadcycles\divide\deadcycles by 2 \showthe\deadcycles\deadcycles=0
\deadcycles=x
\par
\deadcycles=2147483647 \showthe\deadcycles\deadcycles=0
""",
    'insertpenalties': r"""
\T{insertpenalties}
\showthe\insertpenalties
\insertpenalties=5 \showthe\insertpenalties\insertpenalties=0
\skip100=0pt \count100=1000 \dimen100=20pt
\hbox{a}\insert100{\penalty5 \hbox{n}\penalty7 }\showthe\insertpenalties\hbox{b}\insert100{\vrule height 100pt}\showthe\insertpenalties
\output={\showthe\insertpenalties\showthe\outputpenalty\shipout\vbox{\unvbox255 \ifvoid100 \else\box100 \fi}\showthe\insertpenalties}
\vfill\penalty-10000 \showthe\insertpenalties
\setbox0=\hbox{\showthe\insertpenalties\insertpenalties=1 }\showthe\insertpenalties
\begingroup\insertpenalties=7 \endgroup\showthe\insertpenalties
\advance\insertpenalties by 1 \showthe\insertpenalties\insertpenalties=0
\insertpenalties=x
\par
""",
    'patterns': r"""
\T{patterns outside INITEX}
\patterns{1a}
\patterns x
\show\patterns
""",
    'hyphenation': r"""
\T{hyphenation}
\hyphenation{ta-ble}\hyphenation{}\hyphenation{FOO-bar ba-z}\hyphenation{a-b-c-d-e-f-g}
\hyphenation{bad1}
\hyphenation{-}
\hyphenation{--}
\hyphenation{a-}
\hyphenation{\relax}
\hyphenation{\char`a}
\chardef\x=`a \hyphenation{\x b-c}
\hyphenation{xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx}
\hyphenation x
\hyphenation{a b
c}
\setbox1=\vbox{\hsize=10pt \parfillskip=0pt plus 1fil\noindent table foobar baz\par}\showbox1
\begingroup\language=1 \hyphenation{ta-bl-e}\setbox1=\vbox{\hsize=10pt \language=1 \noindent table\par}\showbox1\endgroup
\setbox1=\vbox{\hsize=10pt \language=255 \noindent typesetting\par}\showbox1
\setbox1=\vbox{\hsize=10pt \language=256 \noindent typesetting\par}\showbox1
\lccode`\-=`\- \hyphenation{a--b}\lccode`\-=0
\show\hyphenation
\setbox1=\hbox{\hyphenation{in-box}}\showbox1
\global\hyphenation{g-l}
\catcode`\~=13 \def~{a}\hyphenation{~b}
""",
    'stop': '',
    'errhelp': r"""
\T{errhelp}
\errhelp={single line}\errmessage{e1}
\errhelp={}\errmessage{e2}
\newlinechar=`| \errhelp={one|two|three|four|five|six|seven}\errmessage{e3}\newlinechar=-1
\errhelp={help used once}\errmessage{e4}\errmessage{e5}
\errhelp={}
\begingroup\errhelp={local}\errmessage{e6}\endgroup\errmessage{e7}
\errhelp={\undefinedthing}\errmessage{e8}\errhelp={}
""",
    'errorcontextlines': r"""
\T{errorcontextlines effect}
\def\a{\b}\def\b{\c}\def\c{\d}\def\d{\e}\def\e{\undefinedthing}
\errorcontextlines=-1 \a \errorcontextlines=0 \a \errorcontextlines=1 \a \errorcontextlines=2 \a \errorcontextlines=100 \a
\errorcontextlines=4
""",
    'escapechar': r"""
\T{escapechar effect}
\def\s{\V{\string\a}\show\a\V{\meaning\relax}\undefinedthing}
\def\a{\relax}
\escapechar=-1 \s\escapechar=65 \s\escapechar=256 \s\escapechar=255 \s\escapechar=0 \s\escapechar=`\\ \s
""",
    'newlinechar': r"""
\T{newlinechar effect}
\newlinechar=`| \V{a|b}\message{c|d}\errmessage{e|f}\immediate\write16{g|h}\show\relax
\def\m{x|y}\show\m\showthe\toks0 \newlinechar=-1 \V{a|b}
\newlinechar=256 \V{a|b}\newlinechar=10 \V{a^^Jb}\newlinechar=-1
""",
    'endlinechar': r"""
\T{endlinechar effect}
\def\m{a
b}\show\m
\endlinechar=-1 \def\m{a
b}\show\m \endlinechar=13
\endlinechar=`X \def\m{a
b}\show\m \endlinechar=13
\endlinechar=256 \def\m{a
b}\show\m \endlinechar=13
\catcode`\^^M=12 \endlinechar=13 \def\m{a
b}\show\m \catcode`\^^M=5
""",
    'time': r"""\T{clock}\V{\the\time,\the\day,\the\month,\the\year}""",
    'tracingonline': r"""\T{tracingonline}\tracingonline=0 \showlists\tracingonline=1 \showlists""",
    'tracinglostchars': r"""
\T{tracinglostchars}
\setbox1=\hbox{\tracinglostchars=0 \char200 }\setbox1=\hbox{\tracinglostchars=1 \char201 }
\setbox1=\hbox{\tracinglostchars=2 \char202 }\setbox1=\hbox{\tracinglostchars=3 \char203 }\setbox1=\hbox{\tracinglostchars=4 \char204 }
\setbox1=\hbox{\nullfont\tracinglostchars=3 a}
\setbox1=\hbox{\tracinglostchars=-1 \char205 }
""",
    'mag': r"""
\T{mag}
\showthe\mag\dimen1=1truept \showthe\dimen1
\mag=2000
\dimen1=1truept \showthe\dimen1
\mag=1000
\showthe\mag
\mag=0 \dimen1=1truept \showthe\mag
\mag=32769 \dimen1=1truept \showthe\mag
\mag=-1 \dimen1=1truept \showthe\mag
\mag=1000
""",
    'holdinginserts': r"""
\T{holdinginserts effect}
\holdinginserts=1 \hbox{a}\insert100{\hbox{held}}\vfill\penalty-10000 \holdinginserts=0 \hbox{b}\vfill\penalty-10000
""",
    'maxdeadcycles': r"""
\T{maxdeadcycles effect}
\output={\setbox0=\box255 }\maxdeadcycles=2
\hbox{}\penalty-10000 \hbox{}\penalty-10000 \hbox{}\penalty-10000 \hbox{}\penalty-10000
\deadcycles=0 \maxdeadcycles=25
""",
    'outputpenalty': r"""
\T{outputpenalty}
\output={\showthe\outputpenalty\shipout\box255 }
\hbox{}\penalty5 \vfill\penalty-10000 \hbox{}\penalty-10001 \hbox{}\vfill\penalty-20000 \vsize=20pt \hbox to 10pt{}\hbox to 10pt{}\penalty5 \hbox{}\hbox{}\hbox{}\vfill\penalty-10000 \vsize=120pt
""",
    'output': r"""
\T{output routine paths}
\output={\unvbox255 }\hbox{a}\penalty-10000 \hbox{b}
\output={\setbox0=\box255 \deadcycles=0 }\hbox{a}\penalty-10000
\output={\setbox0=\box255 {\deadcycles=0 }\hbox{a}\penalty-10000 }
\output={\shipout\box255 }}\hbox{b}\penalty-10000
\output={\shipout\box255 }\hbox{c}\penalty-10000
\output={\showlists\shipout\box255 \insert100{x}\hbox{d}\penalty5 }\hbox{e}\penalty-10000
\output={\shipout\box255 }\vfill\penalty-10000
\output={\shipout\box255 \par}\hbox{e}\penalty-10000
\output={\shipout\box255 \begingroup}\hbox{f}\penalty-10000
\endgroup
\output={\global\setbox1=\box255 }\hbox{g}\penalty-10000 \showbox1 \deadcycles=0
\output={\shipout\box255 \ifnum\count21<3 \global\advance\count21 by 1 \hbox{h}\penalty-10000 \fi}\hbox{i}\penalty-10000
\output={\shipout\box255 }
\output=x
\output={}\hbox{j}\penalty-10000 \deadcycles=0
\output={\shipout\box255 }\hbox{k}\vfill\penalty-10000
""",
    'shipout': r"""
\T{shipout}
\shipout\hbox{a}\shipout\vbox{\hbox{b}}\shipout\vtop{\hbox{c}}\shipout\box3 \shipout\copy3
\setbox1=\hbox{d}\shipout\copy1 \showbox1 \shipout\box1 \showbox1
\count0=5 \count1=-3 \count9=2147483647 \shipout\hbox{counters}\count0=0 \count1=0 \count9=0
\count0=-1 \shipout\hbox{}\count0=0
\shipout\hbox to 16383pt{}\shipout\vbox to 16383pt{}\shipout\hbox{\vrule height 16383pt depth 16383pt}
\shipout\hbox{\raise 16383pt\hbox{x}\lower 16383pt\hbox{y}\kern 16383pt z\kern 16383pt}
\shipout\hbox{\kern-16383pt a}\shipout\vbox{\kern-16383pt\hbox{b}}
\shipout\hbox{\vrule width-1pt height-1pt depth-1pt}\shipout\vbox{\hrule width-1pt height-1pt depth-1pt}
\setbox1=\hbox{\shipout\hbox{inner}}\showbox1
\setbox1=\hbox{$\shipout\hbox{in math}$}
\noindent\shipout\hbox{in par}x\par
\shipout\relax
\shipout x\par
\shipout\hrule
\shipout\lastbox
\shipout\vsplit3 to 0pt
\global\shipout\hbox{}
\show\shipout
""",
    'lastskip': '',
}

DVI_BODY = r"""\shipout\vbox{\hrule width 10pt height 3pt\hbox to 100pt{a\special{dvi special}\leaders\hrule\hfil b\cleaders\hbox to 7pt{.\hss}\hfil
  c\xleaders\hbox to 9pt{-\hss}\hfil}\moveright 3pt\hbox{\raise 2pt\hbox{r}\lower 2pt\vbox{\hbox{v}}\tenbf x\tenit y\tenrm z}}
\mag=2000 \shipout\hbox{\kern 1truept x}\mag=1000
\count0=5 \count1=-3 \count9=7 \shipout\hbox{counters}\count0=0 \count1=0 \count9=0
\font\f=cmr10 at 15pt \font\g=cmr10 scaled 1200 \shipout\hbox{\f a\g b\tenrm c\kern 16383pt d}
\shipout\vbox{\kern 1000pt\hbox{far}\kern-1000pt\hbox{back}}
"""

# Settings fixed once output has started.  Each value gets its own document, or only the first could ever be seen.
PER_DOC_VALUES = {
    'pdfoutput': ['0', '1', '-1'], 'pdfminorversion': ['0', '4', '5', '7', '9', '10'], 'pdfmajorversion': ['1', '2', '3'], 'pdfoptionpdfminorversion': ['4', '7'],
    'pdfcompresslevel': ['0', '1', '5', '9', '10', '-1'], 'pdfobjcompresslevel': ['0', '1', '2', '3', '4', '-1'], 'pdfdecimaldigits': ['0', '1', '2', '3', '4', '5', '-1'],
    'pdfdraftmode': ['1', '0'], 'pdfpkresolution': ['0', '72', '8000'], 'pdfimageresolution': ['0', '72', '65536'], 'pdfgamma': ['1000', '0'],
    'pdfsuppressptexinfo': ['-1', '0', '1', '2', '4', '8', '15'], 'pdfinfoomitdate': ['1'], 'pdfomitcharset': ['1'], 'pdfomitinfodict': ['1'], 'pdfomitprocset': ['1', '-1'],
    'pdfptexuseunderscore': ['1'], 'pdfuniqueresname': ['1'], 'pdfgentounicode': ['1'], 'pdfforcepagebox': ['1'], 'pdfpagebox': ['1', '5'],
    'pdfinclusioncopyfonts': ['1'], 'pdfinclusionerrorlevel': ['1', '-1'], 'pdfsuppresswarningpagegroup': ['1'],
}

PNG_PROBE = r"""
\T{PNG}
\def\pg#1{\shipout\vbox{\hsize=200pt \noindent\hbox to 100pt{#1\hfil}}}
\def\im#1{\T{#1}\pdfximage width 1cm {#1}\showthe\pdflastximage\showthe\pdflastximagepages\showthe\pdflastximagecolordepth
  \V{\pdfximagebbox\pdflastximage1,\pdfximagebbox\pdflastximage2,\pdfximagebbox\pdflastximage3,\pdfximagebbox\pdflastximage4}\pg{\pdfrefximage\pdflastximage}}
\im{gray1.png}
\im{gray2.png}
\im{gray4.png}
\im{gray8.png}
\im{gray16.png}
\im{rgb8.png}
\im{rgb16.png}
\im{palette4.png}
\im{palette8.png}
\im{grayalpha8.png}
\im{rgba8.png}
\im{rgba16.png}
\im{gamma.png}
\im{phys.png}
\pdfximage {gray8.png}\pg{\pdfrefximage\pdflastximage}
\pdfximage height 1cm depth 1pt {rgb8.png}\pg{\pdfrefximage\pdflastximage}
\pdfximage attr {/Foo 1} {rgb8.png}\pg{\pdfrefximage\pdflastximage}
\immediate\pdfximage {rgb8.png}\pg{\pdfrefximage\pdflastximage}
\pdfimagehicolor=0 \pdfximage {gray16.png}\pg{\pdfrefximage\pdflastximage}\pdfimagehicolor=1
\pdfimageapplygamma=1 \pdfgamma=2200 \pdfimagegamma=1000 \pdfximage {gamma.png}\pg{\pdfrefximage\pdflastximage}\pdfximage {rgb8.png}\pg{\pdfrefximage\pdflastximage}
\pdfimageapplygamma=0 \pdfgamma=1000
\pdfimageresolution=300 \pdfximage {phys.png}\pg{\pdfrefximage\pdflastximage}\pdfximage {gray8.png}\pg{\pdfrefximage\pdflastximage}\pdfimageresolution=72
\pdfminorversion=4 \pdfximage {rgba8.png}\pg{\pdfrefximage\pdflastximage}
"""

JPEG_PROBE = r"""
\T{JPEG}
\def\pg#1{\shipout\vbox{\hsize=200pt \noindent\hbox to 100pt{#1\hfil}}}
\def\im#1{\T{#1}\pdfximage {#1}\showthe\pdflastximage\showthe\pdflastximagepages\showthe\pdflastximagecolordepth
  \V{\pdfximagebbox\pdflastximage1,\pdfximagebbox\pdflastximage2,\pdfximagebbox\pdflastximage3,\pdfximagebbox\pdflastximage4}\pg{\pdfrefximage\pdflastximage}}
\im{gray.jpg}
\im{rgb.jpg}
\im{cmyk.jpg}
\im{density.jpg}
\im{progressive.jpg}
\pdfimageresolution=300 \im{rgb.jpg}
\im{density.jpg}
\pdfimageresolution=72
\pdfximage width 1cm {cmyk.jpg}\pg{\pdfrefximage\pdflastximage}
"""

PDF_PROBE = r"""
\T{PDF inclusion}
\def\pg#1{\shipout\vbox{\hsize=200pt \noindent\hbox to 100pt{#1\hfil}}}
\def\sh{\showthe\pdflastximage\showthe\pdflastximagepages\showthe\pdflastximagecolordepth\V{\pdfximagebbox\pdflastximage1,\pdfximagebbox\pdflastximage2,\pdfximagebbox\pdflastximage3,\pdfximagebbox\pdflastximage4}\pg{\pdfrefximage\pdflastximage}}
\pdfximage {probe.pdf}\sh
\pdfximage page 1 {probe.pdf}\sh
\pdfximage page 2 {probe.pdf}\sh
\pdfximage page 3 {probe.pdf}\sh
\pdfximage page 3 width 1cm {probe.pdf}\sh
\pdfximage mediabox {probe.pdf}\sh
\pdfximage cropbox {probe.pdf}\sh
\pdfximage bleedbox {probe.pdf}\sh
\pdfximage trimbox {probe.pdf}\sh
\pdfximage artbox {probe.pdf}\sh
\pdfximage page 2 trimbox {probe.pdf}\sh
\pdfximage attr {/Foo 1} {probe.pdf}\sh
\immediate\pdfximage {probe.pdf}\sh
\pdfpagebox=2 \pdfximage {probe.pdf}\sh\pdfpagebox=0
\pdfinclusioncopyfonts=1 \pdfximage {probe.pdf}\sh\pdfinclusioncopyfonts=0
\pdfinclusionerrorlevel=-1 \pdfximage {probe.pdf}\sh\pdfinclusionerrorlevel=0
\pdfximage page 4 {probe.pdf}\sh
\pdfximage page 0 {probe.pdf}\sh
\pdfximage named {x} {probe.pdf}\sh
"""

# Everything here ends the job, on purpose.  Each runs alone so nothing else is hidden.
FATAL = {
    'end-in-group': r'\begingroup\T{x}\end',
    'end-in-groups': r'\begingroup{\begingroup\end',
    'end-in-if': r'\iftrue\end',
    'end-in-ifs': r'\iftrue\unless\iffalse\ifcase0 \ifnum1=1 \end',
    'end-in-group-and-if': r'\begingroup\iftrue\showifs\showgroups\end',
    'end-with-page': r'\hbox{a}\end',
    'end-in-paragraph': r'text\end',
    'end-in-output': r'\output={\end}\hbox{a}\penalty-10000 \end',
    'end-with-inserts': r'\hbox{a}\insert100{\hbox{note}}\end',
    'end-with-holdinginserts': r'\holdinginserts=1 \hbox{a}\insert100{\hbox{note}}\end',
    'end-empty-output': r'\output={\setbox0=\box255 \deadcycles=0 }\hbox{a}\end',
    'end-deadcycles': r'\deadcycles=1 \end',
    'end-in-hbox': r'\setbox0=\hbox{\end}\end',
    'end-in-math': r'$\end$',
    'end-in-display': r'$$\end$$',
    'end-in-alignment': r'\halign{#\cr\end\cr}',
    'end-in-vcenter': r'\setbox0=\hbox{$\vcenter{\end}$}',
    'dump-plain': r'\T{x}\dump',
    'dump-in-group': r'\begingroup\dump',
    'dump-in-paragraph': r'text\dump',
    'errorstopmode-error': r'\errorstopmode\undefinedthing\T{not reached}\end',
    'errorstopmode-show': r'\errorstopmode\show\relax\T{not reached}\end',
    'errorstopmode-pausing': r'\errorstopmode\pausing=1' '\n' r'\T{paused}' '\n' r'\end',
    'read-terminal': r'\read16 to\a\end',
    'read-unopened': r'\read5 to\a\end',
    'read-negative': r'\read-1 to\a\end',
    'readline-terminal': r'\readline16 to\a\end',
    'read-eof-in-group': '\\immediate\\openout1=fatalread.tex\n\\immediate\\write1{\\string{}\n\\immediate\\closeout1\n\\openin1=fatalread\n\\read1 to\\a\\show\\a\n\\end',
    'input-missing': r'\input nosuchfileatall \T{not reached}\end',
    'input-missing-extension': r'\input nosuchfileatall.xyz \end',
    'input-empty': r'\input\relax\end',
    'input-braced-missing': r'\input{nosuchfile}\end',
    'input-absolute-missing': r'\input /nonexistent/directory/file.tex \end',
    'input-parent': r'\input ../nosuchfile.tex \end',
    'input-depth': '\\immediate\\openout1=fataldepth.tex\n\\immediate\\write1{\\string\\input fataldepth}\n\\immediate\\closeout1\n\\input fataldepth\n\\end',
    'openout-parent': r'\immediate\openout1=../coverage-probe-not-written.out \immediate\write1{x}\end',
    'openout-absolute': r'\immediate\openout1=/nonexistent/directory/file.out \end',
    'openout-dotfile': r'\immediate\openout1=.dotfile \immediate\write1{x}\immediate\closeout1 \end',
    'openin-parent': r'\openin1=../nosuchfile.tex \ifeof1 \T{refused}\fi\end',
    'eof-no-end': r'\T{no end follows}',
    'eof-in-def': r'\def\a{',
    'eof-in-def-parameters': r'\def\a#1',
    'eof-in-macro-argument': r'\def\a#1{}\a{',
    'eof-in-delimited-argument': r'\def\a#1.{}\a x',
    'eof-in-text': r'\toks0={',
    'eof-in-preamble': r'\halign{#',
    'eof-in-alignment': r'\halign{#\cr',
    'eof-in-if': r'\iffalse',
    'eof-in-ifcase': r'\ifcase3 ',
    'eof-in-skipped-text': r'\iffalse\if\else\or',
    'eof-in-group': r'{',
    'eof-in-begingroup': r'\begingroup',
    'eof-in-paragraph': r'text',
    'eof-in-math': r'$',
    'eof-in-display': r'$$',
    'eof-in-hbox': r'\hbox{',
    'eof-in-csname': r'\csname a',
    'eof-in-number': r'\count1=',
    'eof-in-write': r'\immediate\write-1{',
    'eof-in-input-file': '\\immediate\\openout1=fatalfile.tex\n\\immediate\\write1{\\string\\def\\string\\a\\string{}\n\\immediate\\closeout1\n\\input fatalfile\n\\end',
    'eof-in-scantokens': r'\scantokens{\def\a{}\end',
    'eof-in-scantokens-macro': r'\def\b#1{}\scantokens{\b}\end',
    'eof-outer-in-def': r'\outer\def\o{}\def\a{\o}\end',
    'eof-outer-in-skip': r'\outer\def\o{}\iffalse\o\fi\end',
    'hundred-errors': r'\count1=0 \loop\ifnum\count1<150 \undefinedthing\advance\count1 by 1 \repeat\T{not reached}\end',
    'hundred-errors-par-resets': r'\count1=0 \loop\ifnum\count1<150 \undefinedthing\par\advance\count1 by 1 \repeat\T{reached}\end',
    'output-loop': r'\output={\setbox0=\box255 }\count1=0 \loop\ifnum\count1<30 \hbox{}\penalty-10000 \advance\count1 by 1 \repeat\T{after}\end',
    'unbalanced-output': r'\output={\shipout\box255 }\hbox{}\penalty-10000 \output={\setbox0=\box255 \deadcycles=0 }}\hbox{}\penalty-10000 \end',
    'infinite-shrink': r'\vsize=1pt \hbox{}\vskip 0pt minus 1fil \hbox{}\penalty-10000 \end',
    'infinite-shrink-split': r'\setbox1=\vbox{\hbox{}\vskip 0pt minus 1fil\hbox{}}\setbox2=\vsplit1 to 1pt \end',
    'infinite-shrink-par': r'\leftskip=0pt minus 1fil \noindent a\par\end',
    'pdfxform-void': r'\pdfxform 1 \T{not reached}\end',
    'pdfxform-big-register': r'\pdfxform 32768 \end',
    'pdfrefxform-none': r'\shipout\hbox{\pdfrefxform 0}\end',
    'pdfrefximage-none': r'\shipout\hbox{\pdfrefximage 0}\end',
    'pdfrefobj-none': r'\shipout\hbox{\pdfrefobj 0}\end',
    'pdfximage-missing': r'\pdfximage {nosuchimage.png}\end',
    'pdfximage-jpg-missing': r'\pdfximage {nosuchimage.jpg}\end',
    'pdfximage-pdf-missing': r'\pdfximage {nosuchimage.pdf}\end',
    'pdfximage-no-extension': r'\pdfximage {nosuchimage}\end',
    'pdfobj-file-missing': r'\immediate\pdfobj file {nosuchfile.txt}\end',
    'pdfobj-stream-file-missing': r'\immediate\pdfobj stream file {nosuchfile.txt}\end',
    'pdfmapline-missing-pfb': r'\pdfmapline{=cmr10 CMR10 <nosuchfontfile.pfb}\shipout\hbox{\tenrm a}\end',
    'pdfmapline-missing-enc': r'\pdfmapline{=cmr10 CMR10 "TeXBase1Encoding ReEncodeFont" <nosuchencoding.enc <cmr10.pfb}\shipout\hbox{\tenrm a}\end',
    'pdfmapline-removed': r'\pdfmapline{-cmr10}\shipout\hbox{\tenrm a}\end',
    'pdfmapline-base14': r'\pdfmapline{=cmr10 Times-Roman}\shipout\hbox{\tenrm a}\end',
    'pdfmapline-no-file-builtin': r'\pdfmapline{=cmr10 CMR10}\shipout\hbox{\tenrm a}\end',
    'pdfmapline-ttf': r'\pdfmapline{=cmr10 CMR10 <nosuchfontfile.ttf}\shipout\hbox{\tenrm a}\end',
    'pdfmapline-otf': r'\pdfmapline{=cmr10 CMR10 <nosuchfontfile.otf}\shipout\hbox{\tenrm a}\end',
    'unmapped-font-pk': r'\font\f=cmr7 at 7pt \pdfmapline{-cmr7}\shipout\hbox{\f a}\end',
    'pdffontexpand-noauto-missing': r'\font\f=cmr10 at 9pt \pdffontexpand\f 20 20 5 \relax \pdfadjustspacing=2 \setbox0=\vbox{\hsize=100pt \f one two three four five six seven eight nine ten eleven twelve\par}\shipout\box0 \end',
    'pdffontexpand-pk-font': r'\pdfmapline{-cmr7}\font\f=cmr7 at 7pt \pdffontexpand\f 20 20 5 autoexpand \relax \pdfadjustspacing=2 \setbox0=\vbox{\hsize=100pt \f one two three four five six seven eight nine ten eleven twelve\par}\shipout\box0 \end',
    'dvi-pdfliteral': r'\pdfoutput=0 \pdfliteral{q Q}\shipout\hbox{a}\end',
    'dvi-pdfximage': r'\pdfoutput=0 \pdfximage{probe.jpg}\end',
    'dvi-pdfobj': r'\pdfoutput=0 \pdfobj {<<>>}\end',
    'dvi-pdfannot': r'\pdfoutput=0 \pdfannot{/A 1}\end',
    'dvi-pdfstartlink': r'\pdfoutput=0 \setbox0=\hbox{\pdfstartlink goto page 1 {/Fit} a\pdfendlink}\shipout\box0 \end',
    'dvi-pdfdest': r'\pdfoutput=0 \pdfdest name {a} fit \end',
    'dvi-pdfoutline': r'\pdfoutput=0 \pdfoutline goto page 1 {/Fit} {a}\end',
    'dvi-pdfcolorstack': r'\pdfoutput=0 \pdfcolorstack 0 push{1 g}\end',
    'dvi-pdfsave': r'\pdfoutput=0 \pdfsave\shipout\hbox{a}\end',
    'dvi-pdfinfo': r'\pdfoutput=0 \pdfinfo{/A (b)}\shipout\hbox{a}\end',
    'dvi-pdfcatalog': r'\pdfoutput=0 \pdfcatalog{/A (b)}\shipout\hbox{a}\end',
    'dvi-pdfmapline': r'\pdfoutput=0 \pdfmapline{=cmr10 CMR10 <cmr10.pfb}\shipout\hbox{a}\end',
    'pdfoutput-after-shipout': r'\shipout\hbox{a}\pdfoutput=0 \shipout\hbox{b}\end',
    'pdfoutput-after-dvi-shipout': r'\pdfoutput=0 \shipout\hbox{a}\pdfoutput=1 \shipout\hbox{b}\end',
    'pdfoutput-after-font-use': r'\pdfoutput=0 \shipout\hbox{a}\pdfoutput=-1 \shipout\hbox{b}\pdfoutput=0 \end',
    'dimen-too-big-page': r'\shipout\vbox to 16383pt{\vskip 16383pt\hbox{a}\vskip 16383pt\hbox{b}}\end',
    'huge-page': r'\pdfpagewidth=16383pt \pdfpageheight=16383pt \shipout\hbox to 16383pt{\hfil a}\end',
    'vcenter-unbalanced': r'\setbox1=\hbox{$\vcenter{$}',
    'file-name-too-long': r'\immediate\openout1=xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx.out \end',
}

# INITEX.  first_line * selects e-TeX mode; without it compatibility mode.
INI = {
    'compat-primitives': ('compat', r"""
\catcode`\{=1 \catcode`\}=2 \catcode`\#=6
\def\T#1{\immediate\write-1{[[#1]]}}
\T{e-TeX primitives are absent in compatibility mode}
\show\numexpr\show\dimexpr\show\unexpanded\show\detokenize\show\protected\show\scantokens\show\readline\show\marks\show\middle
\show\ifdefined\show\ifcsname\show\unless\show\interactionmode\show\lastnodetype\show\TeXXeTstate\show\beginL
\T{pdfTeX primitives are present}
\show\pdfoutput\show\pdfliteral\show\pdfstrcmp\show\expanded\show\ifincsname\show\pdfprimitive\show\quitvmode
\T{knuth primitives}
\show\relax\show\hbox\show\par\show\ \show\/\show\-
\T{initial values}
\showthe\tolerance\showthe\pretolerance\showthe\hsize\showthe\vsize\showthe\baselineskip\showthe\mag\showthe\language\showthe\catcode`\\\showthe\catcode`\^^M
\showthe\catcode`\%\showthe\catcode`a\showthe\catcode`1\showthe\lccode`a\showthe\uccode`a\showthe\sfcode`a\showthe\sfcode`A\showthe\mathcode`a\showthe\mathcode`1\showthe\mathcode`A
\showthe\delcode`.\showthe\escapechar\showthe\endlinechar\showthe\newlinechar\showthe\hyphenchar\nullfont\showthe\skewchar\nullfont\showthe\defaulthyphenchar
\showthe\font\show\nullfont\showthe\pdfoutput\showthe\pdfcompresslevel\showthe\pdfdecimaldigits\showthe\pdfhorigin\showthe\pdfvorigin\showthe\pdfpagewidth
\showthe\pdfpageheight\showthe\pdfgentounicode\showthe\time\showthe\day\showthe\month\showthe\year
\T{end of probes}
\end
"""),
    'etex-mode': ('etex', r"""
\catcode`\{=1 \catcode`\}=2 \catcode`\#=6
\def\T#1{\immediate\write-1{[[#1]]}}
\T{e-TeX mode}
\show\numexpr\show\unexpanded\show\ifdefined\show\beginL\show\TeXXeTstate\showthe\eTeXversion\immediate\write-1{\eTeXrevision}
\showthe\numexpr 1+1\relax
\showthe\tracingassigns\showthe\lastlinefit\showthe\savingvdiscards\showthe\interactionmode
\T{end of probes}
\end
"""),
    'patterns': ('etex', r"""
\catcode`\{=1 \catcode`\}=2 \catcode`\#=6 \catcode`\^=7
\def\T#1{\immediate\write-1{[[#1]]}}
\showboxbreadth=300 \showboxdepth=30 \tracingonline=1 \tracingparagraphs=1 \hbadness=10000
\lccode`a=`a \lccode`b=`b \lccode`c=`c \lccode`d=`d \lccode`e=`e \lccode`t=`t \lccode`y=`y \lccode`p=`p \lccode`s=`s \lccode`i=`i \lccode`n=`n \lccode`g=`g \lccode`o=`o
\catcode`a=11 \catcode`b=11 \catcode`c=11 \catcode`d=11 \catcode`e=11 \catcode`g=11 \catcode`i=11 \catcode`n=11 \catcode`o=11 \catcode`p=11 \catcode`s=11 \catcode`t=11 \catcode`y=11
\T{patterns}
\patterns{a1b b1c 1de .e1 t1y e1t 2s1e 1tt}
\patterns{.ab1 ab1. 1.}
\patterns{a1b}
\patterns{1a2b3c4d5e}
\patterns{9a}
\patterns{\relax}
\patterns{1}
\patterns{}
\patterns{a b

c}
\patterns{-}
\patterns{\char97}
\patterns x
\hyphenation{ty-pe}
\language=1 \patterns{a1c}\hyphenation{ab-cd}\language=0
\language=256 \patterns{a1d}\language=0
\language=255 \patterns{a1e}\language=0
\T{use}
\font\f=cmr10 \f\hyphenchar\f=45 \parfillskip=0pt plus 1fil \tolerance=10000 \hyphenpenalty=0 \lefthyphenmin=1 \righthyphenmin=1 \hsize=5pt
\setbox1=\vbox{\noindent abcde type abde\par}\showbox1
\setbox1=\vbox{\language=1 \noindent abcd acd\par}\showbox1
\T{too late}
\patterns{b1d}
\hyphenation{c-d}
\T{end of probes}
\end
"""),
    'savinghyphcodes': ('etex', r"""
\catcode`\{=1 \catcode`\}=2 \catcode`\#=6
\def\T#1{\immediate\write-1{[[#1]]}}
\showboxbreadth=300 \showboxdepth=30 \tracingonline=1 \hbadness=10000
\lccode`a=`a \lccode`b=`b \catcode`a=11 \catcode`b=11
\savinghyphcodes=1 \language=2 \patterns{a1b}\savinghyphcodes=0 \language=0
\lccode`a=0 \lccode`b=0
\font\f=cmr10 \f\hyphenchar\f=45 \parfillskip=0pt plus 1fil \tolerance=10000 \hyphenpenalty=0 \lefthyphenmin=1 \righthyphenmin=1 \hsize=5pt
\setbox1=\vbox{\language=2 \noindent abab\par}\showbox1
\setbox1=\vbox{\language=0 \noindent abab\par}\showbox1
\T{end of probes}
\end
"""),
    'dump-in-group': ('etex', r"""
\catcode`\{=1 \catcode`\}=2
\begingroup\dump
"""),
    'dump-in-paragraph': ('etex', r"""
\catcode`\{=1 \catcode`\}=2
\font\f=cmr10 \f text\dump
"""),
    'dump-with-if': ('etex', r"""
\iftrue\dump
"""),
    'fonts-and-bye': ('etex', r"""
\catcode`\{=1 \catcode`\}=2 \catcode`\#=6
\font\f=cmr10 \font\g=cmr10 scaled 2000 \textfont0=\f \scriptfont0=\f \scriptscriptfont0=\f
\showboxbreadth=300 \showboxdepth=30 \tracingonline=1
\setbox1=\hbox{\f a\g b}\showbox1 \shipout\box1
\end
"""),
}
INI_FORMATS = {
    'dump-etex': ('etex', r"""
\catcode`\{=1 \catcode`\}=2 \catcode`\#=6 \catcode`\^=7
\def\T#1{\immediate\write-1{[[#1]]}}
\font\f=cmr10 \font\g=cmbx10 scaled 2000 \textfont0=\f \scriptfont0=\f \scriptscriptfont0=\f \f
\everyjob={\T{everyjob}}\def\m{macro}\count1=5 \dimen1=5pt \skip1=5pt \toks1={t}\setbox1=\hbox{\f box}
\lccode`a=`a \patterns{a1b}\hyphenation{a-b}
\T{dumping}
\dump
"""),
    'dump-compat': ('compat', r"""
\catcode`\{=1 \catcode`\}=2
\def\m{macro}
\dump
"""),
}


# Statements that are safe to put on lines of their own: a call of one of the probes' reporting macros with a braced argument.
_STATEMENT = re.compile(r'(?<!\\def)(?<!\\gdef)(?<!\\edef)(?<!\\xdef)\\(?:V|ln|d|r|an|pg|s|g|m)(?=\{)')


# A line with these keeps its statements together: a cut between \\if and \\fi would change what the line means.
_STRUCTURE = re.compile(r'\\(?:if[a-z]*|fi|else|or|unless|begingroup|endgroup|loop|repeat)(?![A-Za-z])')


def split_statements(line):
    """One line of several top-level \\V{..} calls becomes one line each, so that a bad call takes only itself to prim-fatal."""
    if _STRUCTURE.search(line):
        return [line]
    out, start, depth, i = [], 0, 0, 0
    while i < len(line):
        ch = line[i]
        if ch == '\\':
            if depth == 0 and i > start and _STATEMENT.match(line, i):
                out.append(line[start:i])
                start = i
            i += 2
            continue
        if ch == '%':
            break
        if ch == '{':
            depth += 1
        elif ch == '}':
            depth = max(0, depth - 1)
        i += 1
    out.append(line[start:])
    return out


def blocks_of(text):
    """Logical lines of a probe: physical lines joined until the braces balance, so a multi-line definition stays whole."""
    out, cur, depth = [], [], 0
    pieces = []
    for raw in text.split('\n'):
        pieces += split_statements(raw)
    for line in pieces:
        cur.append(line)
        clean = re.sub(r'\\.', '', line)     # \\{ and \\} are not braces, and neither is the } after \\\\
        clean = clean.split('%')[0] if '%' in clean and '\\%' not in clean else clean
        depth += clean.count('{') - clean.count('}')
        if depth <= 0:
            out.append('\n'.join(cur))
            cur, depth = [], 0
    if cur:
        out.append('\n'.join(cur))
    return out


END_TEXT = '\n\\T{end of probes}\n\\end\n'


def reaches_end(blocks, data, work):
    """Does this much of a probe run to its end marker on the oracle, without a fatal error?"""
    d = tempfile.mkdtemp(dir=work)
    try:
        src = os.path.join(d, 'doc.tex')
        with open(src, 'w', encoding='latin-1') as f:
            f.write(PRELUDE + '\n'.join(blocks) + END_TEXT)
        for name, payload in data.items():
            with open(os.path.join(d, name), 'wb') as f:
                f.write(payload)
        rc, log = run_pdftex(src, os.path.join(d, 'o'), timeout=TRIAL_TIMEOUT)
        return rc in (0, 1) and b'[[end of probes]]' in log and b'Fatal error occurred' not in log and b'Emergency stop' not in log
    finally:
        shutil.rmtree(d, ignore_errors=True)


def refine(body, data, work):
    """Move each block that ends the job out of the probe, so the rest of it still runs and still gives a PDF to compare.

    Returns (blocks kept, [(index in the kept list where it was, culprit block)]).  The culprit is found by bisecting on prefixes,
    so it is the first block after which the job no longer reaches its end, whether it died there or ate the rest of the file."""
    blocks = blocks_of(body)
    removed = []
    for _ in range(60):
        if reaches_end(blocks, data, work):
            return blocks, removed
        lo, hi = 0, len(blocks)            # prefix of length lo reaches the end, prefix of length hi does not
        while hi - lo > 1:
            mid = (lo + hi) // 2
            if reaches_end(blocks[:mid], data, work):
                lo = mid
            else:
                hi = mid
        removed.append((hi - 1, blocks[hi - 1]))
        blocks = blocks[:hi - 1] + blocks[hi:]
    raise SystemExit('probe does not settle after 60 removals:\n' + '\n'.join(b for _, b in removed))


def refine_all(bodies, jobs):
    """{name: body} -> {name: (kept blocks, removed)}, cached in target/corpus/gen/prim-plan.json by body hash."""
    os.makedirs(GEN_ROOT, exist_ok=True)
    path = os.path.join(GEN_ROOT, 'prim-plan.json')
    salt = hashlib.sha256((str(PLAN_VERSION) + PRELUDE).encode()).hexdigest()
    try:
        with open(path) as f:
            cache = json.load(f)
    except (OSError, ValueError):
        cache = {}
    work = tempfile.mkdtemp(dir=GEN_ROOT, prefix='.trial-')
    result, used = {}, {}

    def one(item):
        name, (body, data) = item
        key = hashlib.sha256((salt + body).encode('latin-1')).hexdigest()
        got = cache.get(key)
        if got is None:
            kept, removed = refine(body, data, work)
            got = {'kept': kept, 'removed': removed}
            cache[key] = got
        used[key] = got
        return name, got['kept'], got['removed']
    try:
        with cf.ThreadPoolExecutor(jobs) as ex:
            for name, kept, removed in ex.map(one, sorted(bodies.items())):
                result[name] = (kept, removed)
    finally:
        shutil.rmtree(work, ignore_errors=True)
    write_json(path, used)
    return result


def display_name(p):
    return {' ': 'ctrl-space', '-': 'ctrl-hyphen', '/': 'ctrl-slash'}.get(p, p)


IMAGE_PRIMITIVES = ('pdfximage', 'pdfrefximage', 'pdflastximage', 'pdflastximagepages', 'pdflastximagecolordepth', 'pdfximagebbox')


def image_files():
    res = {}
    root = os.path.join(REPO, 'tests', 'ladder')
    with open(os.path.join(root, 'ladder-logo.jpg'), 'rb') as f:
        res['probe.jpg'] = f.read()
    return res


def png(width, height, color_type, depth, rows, plte=None, extra=()):
    """A PNG written by hand, so each colour type and depth exists without an image tool."""
    def chunk(tag, data):
        return struct.pack('>I', len(data)) + tag + data + struct.pack('>I', zlib.crc32(tag + data))
    raw = b''.join(b'\0' + row for row in rows)
    out = b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, depth, color_type, 0, 0, 0))
    for tag, data in extra:
        out += chunk(tag, data)
    if plte:
        out += chunk(b'PLTE', plte)
    return out + chunk(b'IDAT', zlib.compress(raw, 9)) + chunk(b'IEND', b'')


def png_set():
    w, h = 8, 8
    ramp8 = bytes(i * 36 for i in range(8))
    return {
        'gray1.png': png(w, h, 0, 1, [bytes([0xAA if y % 2 else 0x55]) for y in range(h)]),
        'gray2.png': png(w, h, 0, 2, [bytes([0x1B, 0xE4]) for _ in range(h)]),
        'gray4.png': png(w, h, 0, 4, [bytes([0x01, 0x23, 0x45, 0x67]) for _ in range(h)]),
        'gray8.png': png(w, h, 0, 8, [ramp8 for _ in range(h)]),
        'gray16.png': png(w, h, 0, 16, [bytes(b for v in ramp8 for b in (v, v)) for _ in range(h)]),
        'rgb8.png': png(w, h, 2, 8, [bytes(b for v in ramp8 for b in (v, 255 - v, y * 30)) for y in range(h)]),
        'rgb16.png': png(w, h, 2, 16, [bytes(b for v in ramp8 for b in (v, 0, 255 - v, 0, y * 30, 0)) for y in range(h)]),
        'palette4.png': png(w, h, 3, 4, [bytes([0x01, 0x23, 0x01, 0x23]) for _ in range(h)], plte=bytes([255, 0, 0, 0, 255, 0, 0, 0, 255, 9, 9, 9])),
        'palette8.png': png(w, h, 3, 8, [bytes(i % 4 for i in range(8)) for _ in range(h)], plte=bytes([255, 0, 0, 0, 255, 0, 0, 0, 255, 9, 9, 9])),
        'grayalpha8.png': png(w, h, 4, 8, [bytes(b for v in ramp8 for b in (v, 255 - v)) for _ in range(h)]),
        'rgba8.png': png(w, h, 6, 8, [bytes(b for v in ramp8 for b in (v, 0, 255 - v, v)) for _ in range(h)]),
        'rgba16.png': png(w, h, 6, 16, [bytes(b for v in ramp8 for b in (v, 0, 0, 0, 255 - v, 0, v, 0)) for _ in range(h)]),
        'gamma.png': png(w, h, 2, 8, [bytes(b for v in ramp8 for b in (v, v, v)) for _ in range(h)], extra=[(b'gAMA', struct.pack('>I', 45455))]),
        'phys.png': png(w, h, 0, 8, [ramp8 for _ in range(h)], extra=[(b'pHYs', struct.pack('>IIB', 3780, 3780, 1))]),
    }


def jpeg_set(work):
    """JPEGs of each kind pdfTeX distinguishes, made once with ImageMagick and reused.  Returns {} if it is not installed."""
    kinds = {'gray.jpg': ['-size', '16x16', 'gradient:', '-colorspace', 'Gray'],
             'rgb.jpg': ['-size', '16x16', 'gradient:red-blue'],
             'cmyk.jpg': ['-size', '16x16', 'gradient:red-blue', '-colorspace', 'CMYK'],
             'density.jpg': ['-size', '16x16', 'gradient:red-blue', '-density', '300', '-units', 'PixelsPerInch'],
             'progressive.jpg': ['-size', '16x16', 'gradient:red-blue', '-interlace', 'Plane']}
    store = os.path.join(GEN_ROOT, 'jpeg-store')
    os.makedirs(store, exist_ok=True)
    res = {}
    for name, args in kinds.items():
        path = os.path.join(store, name)
        if not os.path.isfile(path):
            tool = shutil.which('magick')
            if not tool:
                return {}
            subprocess.run([tool] + args + ['-strip', '-quality', '90', path], check=True, capture_output=True)
        with open(path, 'rb') as f:
            res[name] = f.read()
    return res


def probe_pdf(work):
    """A small PDF with three pages of different boxes, made by the oracle, for the inclusion probes."""
    d = tempfile.mkdtemp(dir=work)
    try:
        src = os.path.join(d, 'doc.tex')
        with open(src, 'w') as f:
            f.write(r"""\pdftrailerid{}\pdfcompresslevel=0 \pdfobjcompresslevel=0 \pdfpagewidth=200pt \pdfpageheight=100pt
\pdfpageattr={/CropBox [10 10 190 90] /TrimBox [20 20 180 80] /BleedBox [15 15 185 85] /ArtBox [30 30 170 70]}
\shipout\vbox{\hbox{first page \tenbf with fonts}}
\pdfpageattr={/Rotate 90}\shipout\vbox{\hbox{second page}}
\pdfpageattr={}\pdfpagewidth=50pt \pdfpageheight=50pt \shipout\vbox{\hbox{third}}
\end
""")
        run_pdftex(src, os.path.join(d, 'o'), timeout=60)
        with open(os.path.join(d, 'o', 'doc.pdf'), 'rb') as f:
            return f.read()
    finally:
        shutil.rmtree(d, ignore_errors=True)


def generate(jobs=8):
    for name, body in EXTRA_SPECIFIC.items():
        if body:
            SPECIFIC[name] = SPECIFIC.get(name, '') + body
    with open(os.path.join(REPO, 'tests', 'primitives.json')) as f:
        inventory = json.load(f)
    entries = []
    prim_names, bare_names, value_names, fatal_names = [], [], [], []
    images = image_files()
    bodies = {}
    for rec in inventory:
        p = rec['name']
        if rec['class'] != 'enc/ml':
            bodies[p] = (main_body(p), images if p in IMAGE_PRIMITIVES else {})
    pngs = png_set()
    work = tempfile.mkdtemp(dir=GEN_ROOT, prefix='.pdf-')
    try:
        pdf_bytes = probe_pdf(work)
    finally:
        shutil.rmtree(work, ignore_errors=True)
    image_bodies = {'png': (PNG_PROBE, pngs), 'pdf': (PDF_PROBE, {'probe.pdf': pdf_bytes})}
    jpegs = jpeg_set(GEN_ROOT)
    if jpegs:
        image_bodies['jpeg'] = (JPEG_PROBE, jpegs)
    for kind, value in image_bodies.items():
        bodies['*' + kind] = value
    refined = refine_all(bodies, jobs)
    moved, seen_blocks = 0, set()
    for rec in inventory:
        p = rec['name']
        disp = display_name(p)
        if rec['class'] == 'enc/ml':
            if p not in ABSENT:
                raise SystemExit(f'{p}: enc/ml primitive not in the ABSENT list')
            main = write_doc('prim', disp, absent_doc(p))
            prim_names.append(disp)
            entries.append(entry(f'coverage/prim/{disp}', main, notes='must stay undefined: exists only with -enc or -mltex, dropped by DESIGN section 3'))
            continue
        kept, removed = refined[p]
        main = write_doc('prim', disp, PRELUDE + '\n'.join(kept) + END_TEXT)
        prim_names.append(disp)
        data = bodies[p][1]
        for fname, payload in data.items():
            write_doc('prim', disp, payload, filename=fname)
        fields = {}
        if p == 'pdftexbanner':
            fields['notes'] = 'a candidate with its own banner differs here and should nowhere else'
        if removed:
            fields['notes'] = (fields.get('notes', '') + f' {len(removed)} blocks that end the job moved to prim-fatal').strip()
        entries.append(entry(f'coverage/prim/{disp}', main, **fields))
        # What ended the job becomes a document of its own: everything before it, then the block.
        survivors = blocks_of(bodies[p][0])
        k = 0
        for where, block in removed:
            if block not in seen_blocks:
                seen_blocks.add(block)
                k += 1
                name = f'{disp}-{k}'
                fatal_names.append(name)
                doc = write_doc('prim-fatal', name, PRELUDE + '\n'.join(survivors[:where] + [block]) + '\n')
                for fname, payload in data.items():
                    write_doc('prim-fatal', name, payload, filename=fname)
                entries.append(entry(f'coverage/prim-fatal/{name}', doc, timeout=TRIAL_TIMEOUT))
                moved += 1
            survivors = survivors[:where] + survivors[where + 1:]
        if p not in NO_BARE:
            main = write_doc('prim-bare', disp, bare_doc(p))
            bare_names.append(disp)
            entries.append(entry(f'coverage/prim-bare/{disp}', main))
    image_names = []
    for kind, (body, data) in sorted(image_bodies.items()):
        kept, removed = refined['*' + kind]
        image_names.append(kind)
        main = write_doc('prim-image', kind, PRELUDE + '\n'.join(kept) + END_TEXT)
        for fname, payload in data.items():
            write_doc('prim-image', kind, payload, filename=fname)
        note = f'{kind.upper()} inclusion: deferred by DESIGN section 3, so this entry waits for that work' if kind != 'jpeg' else 'JPEG kinds: gray, RGB, CMYK, density, progressive'
        entries.append(entry(f'coverage/prim-image/{kind}', main, notes=note))
        survivors = blocks_of(body)
        for k, (where, block) in enumerate(removed, 1):
            name = f'image-{kind}-{k}'
            fatal_names.append(name)
            doc = write_doc('prim-fatal', name, PRELUDE + '\n'.join(survivors[:where] + [block]) + '\n')
            for fname, payload in data.items():
                write_doc('prim-fatal', name, payload, filename=fname)
            entries.append(entry(f'coverage/prim-fatal/{name}', doc, timeout=TRIAL_TIMEOUT, notes=f'{kind.upper()} inclusion error path'))
            moved += 1
            survivors = survivors[:where] + survivors[where + 1:]
    for p, values in sorted(PER_DOC_VALUES.items()):
        for v in values:
            name = f'{p}={v}'
            value_names.append(name)
            main = write_doc('prim-value', name, value_doc(p, v))
            entries.append(entry(f'coverage/prim-value/{name}', main))
    for name, body in sorted(FATAL.items()):
        text = '% generated by tests/corpus/gen/primitives.py\n\\def\\T#1{\\immediate\\write-1{[[#1]]}}\n\\errorcontextlines=4 \\tracingonline=1 \\tracingcommands=2 \\showboxbreadth=100 \\showboxdepth=100\n' + body + '\n'
        main = write_doc('prim-fatal', name, text)
        fatal_names.append(name)
        if 'probe.jpg' in body:
            write_doc('prim-fatal', name, images['probe.jpg'], filename='probe.jpg')
        entries.append(entry(f'coverage/prim-fatal/{name}', main, timeout=TRIAL_TIMEOUT))
    ini_names = []
    for name, (kind, body) in sorted(INI.items()):
        main = write_doc('prim-ini', name, body)
        ini_names.append(name)
        e = entry(f'coverage/prim-ini/{name}', main, mode='ini', compare='pdf', jobname='doc', cwd=os.path.relpath(os.path.dirname(main), REPO))
        if kind == 'etex':
            del e['main']
            e['first_line'] = '*doc.tex'
        entries.append(e)
    for name, (kind, body) in sorted(INI_FORMATS.items()):
        main = write_doc('prim-ini', name, body)
        ini_names.append(name)
        e = entry(f'coverage/prim-ini/{name}', main, mode='ini-format', jobname='doc', cwd=os.path.relpath(os.path.dirname(main), REPO))
        if kind == 'etex':
            del e['main']
            e['first_line'] = '*doc.tex'
        entries.append(e)
    clean_category('prim', prim_names)
    clean_category('prim-bare', bare_names)
    clean_category('prim-value', value_names)
    clean_category('prim-image', image_names)
    clean_category('prim-fatal', fatal_names)
    clean_category('prim-ini', ini_names)
    no_specific = [r['name'] for r in inventory if r['class'] != 'enc/ml' and r['name'] not in SPECIFIC
                   and r['name'] not in INT + DIMEN + GLUE + MUGLUE + TOKS + PAGE_DIMEN]
    exclusions = [
        {'what': f'{len(ABSENT)} encTeX and MLTeX primitives ({", ".join(ABSENT)}) are tested only as undefined',
         'why': 'They exist only with -enc or -mltex. DESIGN section 3 drops those modes, so the -enc and -mltex paths are not run.'},
        {'what': f'{len(NO_BARE)} primitives have no bare-use document: ' + ' '.join(sorted(NO_BARE)),
         'why': 'A bare use would swallow the rest of the document or end the job. Their probes and prim-fatal entries cover the same paths.'},
        {'what': f'{len(no_specific)} primitives have no hand-written probe in their main document ({", ".join(no_specific)})',
         'why': 'They end the job, so they are only in prim-fatal and prim-ini.'},
        {'what': r'\everyjob at format load is reached only through the system formats; \dump output is judged by its log and by the .fmt existing',
         'why': 'rstex lays out its format differently (DESIGN sections 4 and 7), so .fmt bytes do not compare. A format cannot be loaded across engines.'},
        {'what': 'dropped or deferred features keep entries that a candidate without them cannot pass: coverage/prim-image/png and pdf (deferred), letterspacefont (needs VF), pdfmapline-removed and unmapped-font-pk (PK, Type 3), pdfmapline-ttf and pdfmapline-otf (TrueType, OTF), synctex',
         'why': 'DESIGN section 3. They stay in the denominator, so the count states what is missing instead of hiding it.'},
    ]
    counts = {'prim-main': len(prim_names), 'prim-bare': len(bare_names), 'prim-value': len(value_names), 'prim-image': len(image_names), 'prim-fatal': len(fatal_names),
              'prim-fatal-moved-from-probes': moved, 'prim-ini': len(ini_names), 'prim-inventory': len(inventory)}
    return entries, exclusions, counts


if __name__ == '__main__':
    print(generate()[2])
