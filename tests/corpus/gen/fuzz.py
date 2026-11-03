r"""A seeded differential fuzzer: random but deterministic TeX input, compared on its PDF and at log level with full tracing.

    python3 -I tests/corpus/gen/fuzz.py SEED        print the document for one seed

The generator has its own random number generator (SplitMix64), so a seed gives the same document on any Python.  Output is
line-oriented: every top-level statement sits on one line, which is what lets shrink.py remove lines from a failing case.

Termination is by construction, not by luck.  A macro only calls macros with a lower index, also when redefined, so there is no
recursion, and the expansion cost of each macro is tracked and bounded so that no document blows up exponentially.  \let only
copies from a lower index for the same reason.  A \loop counts on a register nothing else touches.
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))     # python3 -I leaves the script directory off the path
from common import clean_category, entry, write_doc

GENERATOR_VERSION = 1      # bump when output changes for existing seeds
M64 = (1 << 64) - 1
MAX_MACROS = 10
MAX_COST = 60              # bound on macro calls reached from one call


class Rng:
    def __init__(self, seed):
        self.s = (seed * 0x9E3779B97F4A7C15 + 0x243F6A8885A308D3) & M64

    def next(self):
        self.s = (self.s + 0x9E3779B97F4A7C15) & M64
        z = self.s
        z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & M64
        z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & M64
        return z ^ (z >> 31)

    def below(self, n):
        return self.next() % n

    def choice(self, seq):
        return seq[self.below(len(seq))]

    def chance(self, percent):
        return self.below(100) < percent

    def between(self, a, b):
        return a + self.below(b - a + 1)


WORDS = ('the quick brown fox jumps over a lazy dog office affluent difficult waffle typesetting hyphenation paragraph '
         'alignment representation characteristically associativity table baffle shuffle AVA WAW Tokyo understanding '
         'mathematics supercalifragilistic and of in to is as on at').split()
DIMS = '0pt 1pt 2.5pt 3pt 5pt 10pt 12pt 20pt 50pt 100pt -1pt -3.5pt 0.1pt 1in 1cm 3mm 2pc 2bp 3dd 1cc 1em 1ex 16383pt'.split()
GLUES = ('0pt', '1pt', '3pt plus 1pt', '3pt minus 1pt', '3pt plus 1pt minus 1pt', '0pt plus 1fil', '0pt plus 1fill', '0pt plus 1filll',
         '5pt plus 2fil minus 1pt', '0pt minus 1fil', '-2pt plus 1pt', '10pt plus 5pt minus 5pt')
INTS = '0 1 2 3 5 7 10 50 100 150 200 500 1000 2000 5000 9999 10000 -1 -2 -50 -100 -1000 -10000 2147483647 -2147483647'.split()
INT_PARAMS = ('pretolerance tolerance linepenalty hyphenpenalty exhyphenpenalty clubpenalty widowpenalty displaywidowpenalty brokenpenalty binoppenalty '
              'relpenalty predisplaypenalty postdisplaypenalty interlinepenalty doublehyphendemerits finalhyphendemerits adjdemerits delimiterfactor looseness '
              'hbadness vbadness hangafter floatingpenalty lefthyphenmin righthyphenmin uchyph language holdinginserts lastlinefit savingvdiscards').split()
DIM_PARAMS = ('parindent mathsurround lineskiplimit hsize vsize maxdepth splitmaxdepth boxmaxdepth hfuzz vfuzz delimitershortfall nulldelimiterspace scriptspace '
              'overfullrule hangindent emergencystretch').split()
GLUE_PARAMS = 'lineskip baselineskip parskip abovedisplayskip belowdisplayskip abovedisplayshortskip belowdisplayshortskip leftskip rightskip topskip splittopskip tabskip spaceskip xspaceskip parfillskip'.split()
FONTS = r'\tenrm \tenbf \tenit \tensl \tentt \sevenrm \fiverm'.split()
MATH_SYMS = r'a b x y 1 2 + - = < > ( ) , \alpha \beta \sum \int \prod \infty \le \ge \ne \cdot \to \partial'.split()
OUTPUTS = (
    r'\shipout\box255 ',
    r'\shipout\vbox{\unvbox255 \ifvoid100 \else\kern 2pt\box100 \fi}',
    r'\global\setbox5=\box255 \deadcycles=0 ',
    r'\setbox0=\vbox{\unvbox255 \unskip\unpenalty\setbox1=\lastbox}\shipout\box0 ',
    r'\message{[\topmark|\firstmark|\botmark|\topmarks3|\firstmarks3|\botmarks3|\the\outputpenalty|\the\insertpenalties]}\shipout\box255 ',
    r'\ifvoid100 \else\setbox0=\box100 \fi\shipout\box255 \advance\count0 by 1 ',
    r'\unvbox255 \penalty\outputpenalty',
    r'\shipout\box255 \ifnum\count21<3 \global\advance\count21 by 1 \insert100{\hbox{from output}}\hbox{added}\fi ',
)
PLAIN_MACROS = r'\bf \it \rm \sl \tt \quad \qquad \enskip \thinspace \negthinspace \strut \break \nobreak \allowbreak \goodbreak \smallskip \medskip \bigskip \filbreak \hrulefill \dotfill \centerline \leftline \rightline \noindent'.split()


class Gen:
    def __init__(self, seed):
        self.r = Rng(seed)
        self.macros = []          # per index: (index, parameter text kind, cost)
        self.boxes = list(range(10, 18))
        self.counts = list(range(10, 20))
        self.lines = []

    # ------------------------------------------------------------------ small pieces
    def word(self):
        return self.r.choice(WORDS)

    def words(self, lo=1, hi=6):
        return ' '.join(self.word() for _ in range(self.r.between(lo, hi)))

    def dim(self):
        return self.r.choice(DIMS)

    def glue(self):
        return self.r.choice(GLUES)

    def num(self):
        return self.r.choice(INTS)

    def small(self):
        return str(self.r.between(0, 20))

    def box(self):
        return str(self.r.choice(self.boxes))

    def cnt(self):
        return str(self.r.choice(self.counts))

    def call(self, limit, budget):
        """A call of a macro with index below limit whose cost fits, or empty."""
        options = [m for m in self.macros if m[0] < limit and m[2] <= budget[0]]
        if not options:
            return ''
        i, kind, cost = self.r.choice(options)
        budget[0] -= cost
        name = '\\m' + chr(ord('A') + i)
        if kind == 0:
            return name + ' '
        if kind == 1:
            return name + '{' + self.word() + '}'
        if kind == 2:
            return name + '{' + self.word() + '}{' + self.word() + '}'
        if kind == 3:
            return name + ' ' + self.word() + '.' + self.word() + '\\relax '
        return name + ' x' + self.word() + 'y{' + self.word() + '}'

    # ------------------------------------------------------------------ expressions and expansion
    def number_expr(self):
        k = self.r.below(9)
        if k == 0:
            return self.num()
        if k == 1:
            return '\\count' + self.cnt()
        if k == 2:
            return '\\numexpr ' + self.small() + self.r.choice('+-*/') + self.small() + '\\relax '
        if k == 3:
            return '\\numexpr(' + self.num() + self.r.choice('+-') + self.small() + ')*' + self.small() + '/' + self.r.choice(['0', '1', '3', '7']) + '\\relax '
        if k == 4:
            return '"' + self.r.choice(['FF', '7F', '10', '1', 'ABC'])
        if k == 5:
            return "'" + self.r.choice(['17', '7', '100'])
        if k == 6:
            return '`' + self.r.choice(['a', 'Z', r'\a', r'\{', '~'])
        if k == 7:
            return '\\lastpenalty'
        return '\\inputlineno'

    def dim_expr(self):
        k = self.r.below(6)
        if k <= 1:
            return self.dim()
        if k == 2:
            return '\\dimen' + self.cnt()
        if k == 3:
            return '\\dimexpr ' + self.dim() + self.r.choice(['+', '-', '*2/', '*3+']) + self.r.choice(['1pt', '3', '7']) + '\\relax '
        if k == 4:
            return self.r.choice(['\\wd', '\\ht', '\\dp']) + self.box()
        return self.r.choice(['0.5', '2', '1.2']) + '\\baselineskip'

    def expansion(self, budget):
        k = self.r.below(19)
        if k == 14:
            return '\\pdfstrcmp{' + self.word() + '}{' + self.word() + '}'
        if k == 15:
            return '\\pdfescapehex{' + self.word() + '}'
        if k == 16:
            return '\\pdfmdfivesum{' + self.word() + '}'
        if k == 17:
            return '\\expanded{\\the\\count' + self.cnt() + '}'
        if k == 18:
            return '\\ifcsname ' + self.word() + '\\endcsname yes\\else no\\fi '
        if k == 0:
            return '\\the\\count' + self.cnt() + ' '
        if k == 1:
            return '\\number' + self.number_expr() + ' '
        if k == 2:
            return '\\romannumeral' + self.small() + ' '
        if k == 3:
            return '\\string\\relax '
        if k == 4:
            return '\\meaning\\relax '
        if k == 5:
            return '\\jobname '
        if k == 6:
            return '\\the\\dimexpr ' + self.dim() + '\\relax '
        if k == 7:
            return '\\detokenize{' + self.word() + '\\x #}'
        if k == 8:
            return '\\unexpanded{' + self.word() + '\\relax}'
        if k == 9:
            return '\\the\\glueexpr ' + self.glue() + '\\relax '
        if k == 10:
            return '\\ifnum' + self.number_expr() + '<' + self.small() + ' ' + self.word() + '\\else ' + self.word() + '\\fi '
        if k == 11:
            return '\\expandafter\\noexpand\\csname ' + self.word() + '\\endcsname '
        if k == 12:
            return '\\fontname\\font '
        return self.word() + ' '

    # ------------------------------------------------------------------ horizontal material
    def horizontal(self, depth, budget):
        out = []
        for _ in range(self.r.between(1, 5)):
            out.append(self.hitem(depth, budget))
        return ''.join(out)

    def hitem(self, depth, budget):
        k = self.r.below(40 if depth > 0 else 24)
        if k < 8:
            return self.words() + ' '
        if k == 8:
            return r'\-' + self.word() + ' '
        if k == 9:
            return r'\discretionary{' + self.word()[:2] + '}{' + self.word()[:2] + '}{' + self.word()[:3] + '}'
        if k == 10:
            return r'\kern ' + self.dim() + ' '
        if k == 11:
            return r'\hskip ' + self.glue() + ' '
        if k == 12:
            return r'\penalty' + self.num() + ' '
        if k == 13:
            return self.r.choice(FONTS) + ' '
        if k == 14:
            return r'\/'
        if k == 15:
            return self.r.choice([r'\hfil ', r'\hfill ', r'\hss ', r'\hfilneg '])
        if k == 16:
            return r'\vrule width ' + self.r.choice(['0.4pt', '1pt']) + ' height 3pt '
        if k == 17:
            return r'\accent' + self.r.choice(['127', '18', '19', '20', '94']) + ' ' + self.word()[0]
        if k == 18:
            return r'\char' + str(self.r.between(0, 255)) + ' '
        if k == 19:
            return self.expansion(budget)
        if k == 20:
            return r'\mark{' + self.word() + '}'
        if k == 21:
            return r'\marks' + str(self.r.choice([0, 1, 3, 3, 300])) + '{' + self.word() + '}'
        if k == 22:
            return r'\vadjust{\kern ' + self.dim() + '}'
        if k == 23:
            return self.call(MAX_MACROS, budget)
        if k == 24:
            return r'\leaders\hrule\hskip ' + self.glue() + ' '
        if k == 25:
            return r'\cleaders\hbox to ' + self.r.choice(['5pt', '7pt']) + r'{.\hss}\hskip ' + self.glue() + ' '
        if k == 26:
            return r'\xleaders\hbox to 5pt{-\hss}\hskip 20pt plus 1fil '
        if k == 27:
            return '$' + self.math(depth - 1, budget) + '$'
        if k == 28:
            return r'\raise ' + self.dim() + r'\hbox{' + self.horizontal(depth - 1, budget) + '}'
        if k == 29:
            return r'\lower ' + self.dim() + r'\vbox{' + self.vertical(depth - 1, budget) + '}'
        if k == 30:
            return r'\hbox to ' + self.dim() + '{' + self.horizontal(depth - 1, budget) + '}'
        if k == 31:
            return r'\hbox spread ' + self.dim() + '{' + self.horizontal(depth - 1, budget) + '}'
        if k == 32:
            return r'\insert100{\hbox{' + self.word() + '}}'
        if k == 33:
            return '{' + self.horizontal(depth - 1, budget) + '}'
        if k == 34:
            return self.conditional(lambda: self.horizontal(depth - 1, budget))
        if k == 35:
            return r'\unhcopy' + self.box() + ' '
        if k == 36:
            return r'\unskip\unkern\unpenalty '
        if k == 37:
            return r'\setbox' + self.box() + r'=\lastbox '
        if k == 38:
            return r'\noboundary ' + self.word() + ' '
        return r'\aftergroup\relax '

    # ------------------------------------------------------------------ math
    def math(self, depth, budget):
        out = []
        for _ in range(self.r.between(1, 5)):
            out.append(self.matom(depth, budget))
        return ''.join(out)

    def mfield(self, depth, budget):
        return '{' + self.math(depth - 1, budget) + '}' if depth > 0 else self.r.choice(MATH_SYMS) + ' '

    def matom(self, depth, budget):
        k = self.r.below(26 if depth > 0 else 8)
        if k < 5:
            return self.r.choice(MATH_SYMS) + ' '
        if k == 5:
            return self.mfield(depth, budget) + '^' + self.mfield(depth, budget)
        if k == 6:
            return self.mfield(depth, budget) + '_' + self.mfield(depth, budget)
        if k == 7:
            return self.r.choice([r'\,', r'\;', r'\!', r'\quad '])
        if k == 8:
            return '{' + self.math(depth - 1, budget) + self.r.choice([r'\over ', r'\atop ', r'\above 1pt ', r'\overwithdelims() ', r'\atopwithdelims[] ', r'\abovewithdelims()1pt ']) + self.math(depth - 1, budget) + '}'
        if k == 9:
            return r'\left' + self.r.choice(['(', '[', '.', r'\{', '|']) + self.math(depth - 1, budget) + (r'\middle| ' + self.math(depth - 1, budget) if self.r.chance(20) else '') + r'\right' + self.r.choice([')', ']', '.', r'\}', '|'])
        if k == 10:
            return r'\sqrt' + self.mfield(depth, budget)
        if k == 11:
            return r'\mathchoice{a}{b}{c}{d}'
        if k == 12:
            return r'\mathop{\rm ' + self.word() + r'}' + self.r.choice([r'\limits', r'\nolimits', r'\displaylimits']) + '^1_2'
        if k == 13:
            return r'\sum' + self.r.choice([r'\limits', r'\nolimits', '']) + '_{i}^{n}'
        if k == 14:
            return self.r.choice([r'\mathord', r'\mathbin', r'\mathrel', r'\mathopen', r'\mathclose', r'\mathpunct', r'\mathinner', r'\underline', r'\overline']) + self.mfield(depth, budget)
        if k == 15:
            return self.r.choice([r'\mathaccent"7014 ', r'\mathaccent"705E ', r'\hat ', r'\vec ', r'\bar ']) + self.mfield(depth, budget)
        if k == 16:
            return r'\radical"270370 ' + self.mfield(depth, budget)
        if k == 17:
            return r'\mskip ' + self.r.choice(['1mu', '3mu plus 1mu', r'\thinmuskip']) + ' '
        if k == 18:
            return r'\mkern ' + self.r.choice(['1mu', '-1mu', '18mu']) + ' '
        if k == 19:
            return r'\vcenter{' + self.vertical(depth - 1, budget) + '}'
        if k == 20:
            return self.r.choice([r'\displaystyle ', r'\textstyle ', r'\scriptstyle ', r'\scriptscriptstyle '])
        if k == 21:
            return r'\hbox{' + self.horizontal(depth - 1, budget) + '}'
        if k == 22:
            return r'\nonscript\mskip 1mu '
        if k == 23:
            return r'\mathchar"' + self.r.choice(['0141', '7161', '2201', '3224']) + ' '
        if k == 24:
            return r'\fam=' + self.r.choice(['0', '1', '-1']) + ' '
        return self.conditional(lambda: self.math(depth - 1, budget))

    def display(self, depth, budget):
        body = self.math(depth, budget)
        tail = self.r.choice(['', '', r'\eqno(1)', r'\leqno(2)'])
        return '$$' + body + tail + '$$'

    # ------------------------------------------------------------------ vertical material
    def vertical(self, depth, budget):
        out = []
        for _ in range(self.r.between(1, 4)):
            out.append(self.vitem(depth, budget))
        return ''.join(out)

    def vitem(self, depth, budget):
        k = self.r.below(24 if depth > 0 else 10)
        if k < 3:
            return r'\hbox{' + self.words() + '}'
        if k == 3:
            return r'\kern ' + self.dim() + ' '
        if k == 4:
            return r'\vskip ' + self.glue() + ' '
        if k == 5:
            return r'\penalty' + self.num() + ' '
        if k == 6:
            return r'\hrule height ' + self.r.choice(['0.4pt', '1pt']) + ' '
        if k == 7:
            return r'\mark{' + self.word() + '}'
        if k == 8:
            return r'\marks3{' + self.word() + '}'
        if k == 9:
            return self.r.choice([r'\vfil ', r'\vfill ', r'\vss ', r'\vfilneg '])
        if k == 10:
            return r'\vbox{' + self.vertical(depth - 1, budget) + '}'
        if k == 11:
            return r'\vtop to ' + self.dim() + '{' + self.vertical(depth - 1, budget) + '}'
        if k == 12:
            return self.words(5, 25) + r'\par '
        if k == 13:
            return r'\insert100{\hbox{' + self.word() + r'}\penalty ' + self.small() + ' }'
        if k == 14:
            return r'\moveleft ' + self.dim() + r'\hbox{' + self.horizontal(depth - 1, budget) + '}'
        if k == 15:
            return r'\moveright ' + self.dim() + r'\vbox{' + self.vertical(depth - 1, budget) + '}'
        if k == 16:
            return r'\leaders\hrule\vskip ' + self.glue() + ' '
        if k == 17:
            return r'\cleaders\vbox to 5pt{\hbox{.}\vss}\vskip 20pt plus 1fil '
        if k == 18:
            return r'\copy' + self.box() + ' '
        if k == 19:
            return r'\unvcopy' + self.box() + ' '
        if k == 20:
            return r'\unskip\unkern\unpenalty '
        if k == 21:
            return self.conditional(lambda: self.vertical(depth - 1, budget))
        if k == 22:
            return self.alignment(depth - 1, budget)
        return self.call(MAX_MACROS, budget)

    # ------------------------------------------------------------------ structures
    def conditional(self, body):
        k = self.r.below(12)
        yes = body()
        no = body() if self.r.chance(50) else ''
        tail = (r'\else ' + no if no else '') + r'\fi '
        if k == 0:
            return r'\ifnum' + self.number_expr() + self.r.choice('<=>') + self.number_expr() + ' ' + yes + tail
        if k == 1:
            return r'\ifdim' + self.dim_expr() + self.r.choice('<=>') + self.dim_expr() + ' ' + yes + tail
        if k == 2:
            return r'\ifx\mA\mB ' + yes + tail
        if k == 3:
            return r'\if' + self.r.choice(['aa', 'ab', r'\relax\relax', r'a\relax']) + ' ' + yes + tail
        if k == 4:
            return r'\ifcat' + self.r.choice(['ab', 'a1', r'\relax\relax']) + ' ' + yes + tail
        if k == 5:
            return r'\ifodd' + self.number_expr() + ' ' + yes + tail
        if k == 6:
            return self.r.choice([r'\ifvmode ', r'\ifhmode ', r'\ifmmode ', r'\ifinner ']) + yes + tail
        if k == 7:
            return self.r.choice([r'\ifvoid', r'\ifhbox', r'\ifvbox']) + self.box() + ' ' + yes + tail
        if k == 8:
            return r'\ifcase' + self.small() + ' ' + yes + r'\or ' + no + r'\or ' + self.word() + r'\else ' + self.word() + r'\fi '
        if k == 9:
            return r'\unless\ifnum' + self.number_expr() + '<' + self.small() + ' ' + yes + tail
        if k == 10:
            return r'\ifdefined\mC ' + yes + tail
        return self.r.choice([r'\iftrue ', r'\iffalse ']) + yes + tail

    def alignment(self, depth, budget):
        columns = self.r.between(1, 4)
        tmpl = ['#' if self.r.chance(50) else r'\hfil#\hfil' for _ in range(columns)]
        pre = '&'.join(tmpl)
        if self.r.chance(20):
            pre = r'\tabskip=' + self.glue() + ' ' + pre
        if self.r.chance(15):
            pre = pre + r'\tabskip=1pt&&#'
        spec = self.r.choice(['', '', ' to ' + self.r.choice(['100pt', '200pt']), ' spread 10pt'])
        rows = []
        for _ in range(self.r.between(1, 4)):
            cells = []
            for c in range(self.r.between(1, columns)):
                k = self.r.below(8)
                if k == 0:
                    cells.append(r'\omit ' + self.word())
                elif k == 1 and c + 1 < columns:
                    cells.append(self.word() + r'\span ' + self.word())
                elif k == 2:
                    cells.append(self.horizontal(depth, budget))
                else:
                    cells.append(self.word())
            rows.append('&'.join(cells) + r'\cr ')
            if self.r.chance(25):
                rows.append(r'\noalign{' + self.r.choice([r'\hrule ', r'\kern 3pt ', r'\penalty 5 ', r'\vskip 2pt plus 1pt ']) + '}')
        return r'\halign' + spec + '{' + pre + r'\cr ' + ''.join(rows) + '}'

    def valignment(self, depth, budget):
        rows = ''.join(r'\hbox{' + self.word() + r'}&\hbox{' + self.word() + r'}\cr ' for _ in range(self.r.between(1, 3)))
        return r'\valign' + self.r.choice(['', ' to 50pt']) + r'{#\vfil\tabskip=1pt&#\cr ' + rows + '}'

    def paragraph(self, depth, budget):
        parts = []
        for _ in range(self.r.between(1, 6)):
            parts.append(self.horizontal(depth, budget))
            if self.r.chance(15):
                parts.append(self.display(1, budget))
        return ''.join(parts) + r'\par'

    # ------------------------------------------------------------------ top-level statements
    def define_macro(self):
        full = len(self.macros) >= MAX_MACROS
        i = self.r.below(MAX_MACROS) if full else len(self.macros)
        kind = self.r.below(5)
        budget = [MAX_COST // 2]
        params = ['', '#1', '#1#2', '#1.#2\\relax', 'x#1y#2'][kind]
        body = []
        for _ in range(self.r.between(1, 4)):
            t = self.r.below(7)
            if t == 0:
                body.append(self.word() + ' ')
            elif t == 1 and kind:
                body.append('#1')
            elif t == 2:
                c = self.call(i, budget)
                body.append(c)
            elif t == 3:
                body.append(self.expansion(budget))
            elif t == 4:
                body.append(self.hitem(1, budget))
            elif t == 5:
                body.append(r'\begingroup ' + self.word() + r'\endgroup ')
            else:
                body.append(self.conditional(lambda: self.word() + ' '))
        cost = 1 + (MAX_COST // 2 - budget[0])
        rec = (i, kind, cost)
        if full:
            self.macros[i] = (i, kind, max(cost, self.macros[i][2]))
        else:
            self.macros.append(rec)
        prefix = self.r.choice(['', '', '', r'\long', r'\protected', r'\global', r'\long\global'])
        d = self.r.choice([r'\def', r'\def', r'\def', r'\edef', r'\gdef', r'\xdef'])
        name = '\\m' + chr(ord('A') + i)
        return f'{prefix}{d}{name}{params}{{{"".join(body)}}}'

    def statement(self):
        budget = [MAX_COST]
        k = self.r.below(76)
        if k < 5:
            return self.define_macro()
        if k < 8:
            return self.call(MAX_MACROS, budget) + r'\par'
        if k < 16:
            return self.paragraph(2, budget)
        if k < 19:
            return self.vertical(2, budget)
        if k < 21:
            return self.display(3, budget)
        if k < 24:
            return self.alignment(2, budget)
        if k == 24:
            return r'\setbox' + self.box() + r'=\hbox{' + self.valignment(1, budget) + '}'
        if k < 28:
            b = self.box()
            return rf'\setbox{b}=' + self.r.choice([r'\hbox', r'\hbox to 100pt', r'\hbox spread 5pt']) + '{' + self.horizontal(2, budget) + rf'}}\showbox{b} '
        if k < 31:
            b = self.box()
            return rf'\setbox{b}=' + self.r.choice([r'\vbox', r'\vtop', r'\vbox to 80pt']) + '{' + self.vertical(2, budget) + rf'}}\showbox{b} '
        if k == 31:
            a, b = self.box(), self.box()
            return rf'\setbox{a}=\vbox{{\hsize=100pt ' + self.words(10, 30) + rf'\par}}\setbox{b}=\vsplit{a} to ' + self.r.choice(['10pt', '24pt', '50pt', '0pt']) + rf' \showbox{a}\showbox{b}\message{{[\splitfirstmark|\splitbotmark]}}'
        if k == 32:
            return r'\begingroup ' + self.r.choice(['\\' + p + '=' + self.num() for p in INT_PARAMS]) + ' ' + self.paragraph(1, budget) + r'\endgroup'
        if k == 33:
            return ' \\' + self.r.choice(INT_PARAMS) + '=' + self.num() + ' '
        if k == 34:
            return '\\' + self.r.choice(DIM_PARAMS) + '=' + self.dim() + ' '
        if k == 35:
            return '\\' + self.r.choice(GLUE_PARAMS) + '=' + self.glue() + ' '
        if k == 36:
            return r'\begingroup\hsize=' + self.r.choice(['50pt', '100pt', '150pt']) + r'\parshape=' + self.r.choice(['0 ', '1 5pt 80pt ', '2 0pt 80pt 10pt 60pt ', '3 0pt 80pt 10pt 60pt 20pt 40pt ']) + self.paragraph(1, budget) + r'\endgroup'
        if k == 37:
            return r'\hangindent=' + self.r.choice(['10pt', '-10pt']) + r' \hangafter=' + self.r.choice(['1', '-1', '2', '-2']) + ' ' + self.paragraph(1, budget)
        if k == 38:
            return r'\output={' + self.r.choice(OUTPUTS) + '}'
        if k == 39:
            return r'\vfill\penalty-' + self.r.choice(['10000', '20000', '10001'])
        if k == 40:
            return r'\showlists'
        if k == 41:
            return r'\showthe' + self.r.choice([r'\pagegoal', r'\pagetotal', r'\pagedepth', r'\prevdepth', r'\deadcycles', r'\insertpenalties', r'\badness', r'\lastnodetype', r'\currentgrouplevel', r'\currentiflevel', r'\inputlineno'])
        if k == 42:
            return r'\hyphenation{' + '-'.join(self.r.choice(['ta', 'ble', 'of', 'fice', 'ty', 'pe', 'set']) for _ in range(self.r.between(2, 3))) + '}'
        if k == 43:
            return r'\begingroup\hsize=' + self.r.choice(['10pt', '30pt']) + r'\tolerance=10000 \hyphenpenalty=' + self.r.choice(['0', '50', '-1000']) + r'\lefthyphenmin=' + self.small() + r' \noindent ' + self.words(4, 12) + r'\par\endgroup'
        if k == 44:
            c = self.cnt()
            return rf'\count{c}=' + self.number_expr() + rf' \advance\count{c} by ' + self.number_expr() + rf' \multiply\count{c} by ' + self.small() + rf' \divide\count{c} by ' + self.r.choice(['0', '1', '3']) + rf' \showthe\count{c}'
        if k == 45:
            c = self.cnt()
            return rf'\dimen{c}=' + self.dim_expr() + rf' \advance\dimen{c} by ' + self.dim_expr() + rf' \multiply\dimen{c} by ' + self.small() + rf' \showthe\dimen{c}'
        if k == 46:
            c = self.cnt()
            return rf'\skip{c}=' + self.glue() + rf' \advance\skip{c} by ' + self.glue() + rf' \showthe\skip{c}\message{{\the\gluestretch\skip{c},\the\glueshrink\skip{c},\the\gluestretchorder\skip{c}}}'
        if k == 47:
            return r'\edef\zz{' + self.expansion(budget) + self.expansion(budget) + r'}\show\zz'
        if k == 48:
            return r'\count20=0 \loop\ifnum\count20<' + str(self.r.between(1, 6)) + r' ' + self.horizontal(1, budget) + r'\advance\count20 by 1 \repeat\par'
        if k == 49:
            return r'\immediate\write-1{' + self.expansion(budget) + self.expansion(budget) + '}'
        if k == 50:
            return r'\message{' + self.expansion(budget) + '}'
        if k == 51:
            return r'\shipout\hbox{' + self.horizontal(2, budget) + '}'
        if k == 52:
            return r'\shipout\vbox{' + self.vertical(2, budget) + '}'
        if k == 53:
            return r'\futurelet\zz\show\zz ' + self.word()
        if k == 54:
            self.r.below(max(1, len(self.macros)))
            return r'\expandafter\def\csname ' + self.word() + r'\endcsname{' + self.word() + r'}\expandafter\show\csname ' + self.word() + r'\endcsname'
        if k == 55:
            return r'{' + self.r.choice([r'\aftergroup\relax ', r'\afterassignment\relax\count10=1 ', r'\showgroups ', r'\showifs ']) + self.paragraph(1, budget) + '}'
        if k == 56:
            return self.conditional(lambda: self.paragraph(1, budget))
        if k == 57:
            # Errors, on purpose: recovery has to match too.
            return self.r.choice([r'\undefinedcs ', r'}', r'\fi ', r'\or ', r'\else ', r'\endgroup ', r'\count10=x ', r'\dimen10=1 ', r'\hbox x\par', r'\moveleft 1pt x\par', r'$\par',
                                  r'\halign{#\cr a&b\cr}', r'\halign{x\cr}', r'\def\a#2{}', r'\def\a#1#3{}', r'\setbox300000=\hbox{}', r'\vsplit 10', r'\unhbox 300000 ', r'\accent 300 ', r'\char 300 ',
                                  r'\mathchar"8000 ', r'\left x', r'\right)', r'\over ', r'\eqno ', r'\omit ', r'\span ', r'\cr ', r'\noalign{}', r'\mark x\par', r'\insert 300{}', r'\insert 100 x\par',
                                  r'\parshape=1 ', r'\fontdimen 100000\tenrm=1pt ', r'\catcode 300=1 ', r'\lccode`a=300 ', r'\sfcode`a=40000 ', r'\delcode`a=20000000 ', r'\mathcode`a="9000 ', r'\font\x=nosuchfont '])
        if k == 58:
            return r'\global\setbox' + self.box() + r'=\box' + self.box() + ' '
        if k == 60:
            return '\\' + self.r.choice(['everypar', 'everymath', 'everydisplay', 'everyhbox', 'everyvbox']) + r'={\message{every}}'
        if k == 61:
            return r'\everypar={}\everymath={}\everydisplay={}\everyhbox={}\everyvbox={}\everycr={\noalign{\kern 1pt}}'
        if k == 62:
            return r'\toks5={' + self.word() + r'#\relax}\edef\zz{\the\toks5}\show\zz\showthe\toks5 '
        if k == 63:
            return self.r.choice([r'\chardef\zc=65 \show\zc\setbox10=\hbox{\zc}', r'\mathchardef\zc="7161 \setbox10=\hbox{$\zc$}', r'\countdef\zc=30 \zc=5 \showthe\zc', r'\dimendef\zc=30 \zc=5pt \showthe\zc', r'\skipdef\zc=30 \zc=5pt plus 1fil \showthe\zc', r'\toksdef\zc=30 \zc={t}\showthe\zc', r'\muskipdef\zc=30 \zc=5mu \showthe\zc'])
        if k == 64:
            return self.r.choice([r'\uppercase{', r'\lowercase{']) + self.words() + r'\par}'
        if k == 65:
            return self.r.choice([r'\fontdimen2\tenrm=' + self.dim(), r'\hyphenchar\tenrm=' + self.r.choice(['-1', '45', '127']), r'\skewchar\teni=' + self.r.choice(['-1', '127']), r'\showthe\fontdimen' + self.small() + r'\tenrm']) + ' '
        if k == 66:
            return r'\scantokens{' + self.words() + r'\par}'
        if k == 67:
            return r'\interlinepenalties=' + self.r.choice(['0 ', '1 50 ', '2 100 200 ']) + r'\clubpenalties=' + self.r.choice(['0 ', '1 300 ']) + r'\widowpenalties=' + self.r.choice(['0 ', '2 400 500 ']) + self.paragraph(1, budget)
        if k == 68:
            return r'\begingroup\lastlinefit=' + self.r.choice(['500', '1000']) + r' \parfillskip=0pt plus 1fil ' + self.paragraph(1, budget) + r'\endgroup'
        if k == 69:
            return r'\begingroup\font\zf=cmr10 at 11pt \pdffontexpand\zf 20 20 5 autoexpand \rpcode\zf`.=500 \lpcode\zf`A=100 \pdfprotrudechars=2 \pdfadjustspacing=2 \zf\hsize=100pt \noindent ' + self.words(10, 30) + r'.\par\endgroup'
        if k == 70:
            return r'\setbox' + self.box() + r'=\hbox{\pdfsave\pdfliteral{q Q}\pdfrestore\pdfcolorstack0 push{1 g}x\pdfcolorstack0 pop}\showbox' + self.box()
        if k == 71:
            return r'\savingvdiscards=1 \setbox10=\vbox{\hbox{a}\penalty0 \vskip 2pt\hbox{b}}\setbox11=\vsplit10 to 5pt \setbox12=\vbox{\splitdiscards}\showbox12 \savingvdiscards=0 '
        if k == 72:
            return r'\noindent\vadjust pre{\kern ' + self.dim() + r'}' + self.words() + r'\par'
        if k == 73:
            return r'\prevdepth=' + self.dim() + r' \hbox{' + self.word() + r'}\showthe\prevdepth'
        if k == 74:
            return r'\noindent\spacefactor=' + self.r.choice(['1', '999', '1000', '3000', '32767']) + r' ' + self.word() + r' \showthe\spacefactor\par'
        if k == 75:
            return r'\pdfdest name {' + self.word() + r'} fit \pdfannot width 1pt {/A 1}\hbox{\pdfstartlink goto page 1 {/Fit} ' + self.word() + r'\pdfendlink}'
        return r'\TeXXeTstate=1 \setbox' + self.box() + r'=\hbox{\beginR ' + self.horizontal(1, budget) + r'\endR}\TeXXeTstate=0 '

    def document(self, seed):
        out = [f'% coverage fuzz document, generator version {GENERATOR_VERSION}, seed {seed}',
               r'\catcode`\~=13 \def~{\penalty10000 \ }',
               r'\hsize=' + self.r.choice(['200pt', '300pt', '100pt']) + r' \vsize=' + self.r.choice(['100pt', '200pt', '400pt']) + r' \parindent=10pt \tolerance=' + self.r.choice(['200', '1000', '10000']) + r' \hfuzz=0.1pt \vfuzz=0.1pt',
               r'\skip100=6pt plus 1pt \count100=1000 \dimen100=50pt \baselineskip=12pt \lineskip=1pt \lineskiplimit=0pt \topskip=10pt \maxdepth=2pt',
               r'\def\mA{a}\def\mB{a}\def\mC{a}',
               r'\output={' + self.r.choice(OUTPUTS) + '}',
               r'\showboxbreadth=500 \showboxdepth=50 \errorcontextlines=3 \tracingonline=1 \tracingcommands=2 \tracingmacros=2 \tracingoutput=1 \tracingpages=1 \tracingparagraphs=1',
               r'\tracingrestores=1 \tracinglostchars=1 \tracingassigns=1 \tracinggroups=1 \tracingifs=1 \tracingscantokens=1 \tracingnesting=2']
        self.macros = [(0, 0, 1), (1, 0, 1), (2, 0, 1)]
        for _ in range(self.r.between(15, 40)):
            s = self.statement()
            if s:
                out.append(s)
        out.append(r'\end')
        return '\n'.join(out) + '\n'


def document(seed):
    return Gen(seed).document(seed)


def generate(seeds=2000):
    entries, names = [], []
    for seed in range(seeds):
        name = f'{seed:05d}'
        names.append(name)
        main = write_doc('fuzz', name, document(seed))
        entries.append(entry(f'coverage/fuzz/{name}', main, timeout=30, notes=f'seed {seed}'))  # 1882 of 2000 ship pages: judged on the PDF as well as the log, which gate_log keeps
    clean_category('fuzz', names)
    exclusions = [{'what': f'the fuzzer runs {seeds} seeds, 0 to {seeds - 1}',
                   'why': 'A budget, not a limit: tests/corpus/gen/all.py --seeds N runs more. Failing cases shrink with tests/corpus/gen/shrink.py.'}]
    return entries, exclusions, {'fuzz': seeds}


if __name__ == '__main__':
    sys.stdout.write(document(int(sys.argv[1])))
