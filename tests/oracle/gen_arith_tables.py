#!/usr/bin/env python3
"""Expected results for arith.rs, Part 7 and 7b of the WEB, taken from the oracle.

Run with python3 -I.  It writes crates/engine/testdata/arith_oracle.tsv, read by crates/engine/tests/arith.rs.

Each `grid` row is one operation the oracle can be made to perform through a primitive, over the product of
its axes, with the results in order, comma-separated; a result followed by ! is one where the oracle reported
an error:

  x_over_n   x n             \\count1=x \\divide\\count1 by n: §106, §1418
  mult_int   n x             \\count1=n \\multiply\\count1 by x: §105, §1418
  nx_plus_y  n x             \\dimen1=n sp \\multiply\\dimen1 by x: §105, §1418
  units      int digits unit \\dimen1=int.digits unit: §102, §107, §478-§486
  true       mag int digits unit   the same with `true`, after \\mag=mag: §483
  internal   sign a digits v \\dimen1=sign a.digits\\wd1, with \\wd1=v: §481, §105, §107
  mkern      q x             \\mkern, with \\fontdimen6 of family 2 q: §879, §893, the remainder of §106
  mskip      q w             \\mskip, with a \\muskip of width w: §892
  badness    t s             \\hbox to t sp{\\hskip 0pt plus s sp}, then \\badness: §108, §836
  bad_tight  t s             \\hbox to -t sp{\\hskip 0pt minus s sp}, then \\badness: §108, §843
  half       delta axis      \\vcenter, §912, with \\fontdimen22 axis: §100
  scaled     v               \\the\\dimen1 with \\dimen1=v sp: §103
  decimals   digits          \\dimen1=0.digits pt: §102

`random_ops` lists what a `random` row does after \\pdfsetrandomseed: \\pdfuniformdeviate x, or n for
\\pdfnormaldeviate, §124-§127.  Each `random` row is a seed and the results.

`sweep` rows hold the FNV-1a digest of many results, one per line, so that the table stays small.  Their
inputs come from an LCG both sides compute.  A digest row says only that something differs: run this script
with --dump NAME to write that sweep's rows to standard output, and compare.

Each run is `pdftex -ini -etex`, in a temporary directory.
"""
import fractions
import os
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(ROOT, "crates", "engine", "testdata", "arith_oracle.tsv")

MAXD = 0o7777777777  # 2^30-1, \maxdimen
MAXI = 0o17777777777  # 2^31-1
M64 = (1 << 64) - 1
UNITS = ["pt", "in", "pc", "cm", "mm", "bp", "dd", "cc", "nd", "nc", "sp"]

PREAMBLE = r"""\catcode`\{=1 \catcode`\}=2 \catcode`\#=6 \catcode`\$=3
\batchmode \errorcontextlines=-1 \hbadness=10000 \vbadness=10000 \hfuzz=1073741823sp \vfuzz=1073741823sp
\font\s=cmsy10 \font\e=cmex10 \mathsurround=0pt
\textfont2=\s \scriptfont2=\s \scriptscriptfont2=\s \textfont3=\e \scriptfont3=\e \scriptscriptfont3=\e
\fontdimen5\s=0pt \fontdimen6\s=0pt \fontdimen22\s=0pt
\def\R#1{\immediate\write-1{=#1}\noindent\par}
"""


class Lcg:
    """The same generator is in crates/engine/tests/arith.rs."""

    def __init__(self, seed):
        self.x = seed & M64

    def next(self):
        self.x = (self.x * 6364136223846793005 + 1442695040888963407) & M64
        return self.x >> 33

    def nonneg(self):
        """Up to 2^31-1, of every magnitude."""
        r = self.next()
        return r >> (self.next() % 31)

    def mixed(self):
        v = self.nonneg()
        return -v if self.next() & 1 else v

    def dimen(self):
        """|v| at most \\maxdimen."""
        v = self.nonneg() & MAXD
        return -v if self.next() & 1 else v

    def digits(self, most):
        n = self.next() % (most + 1)
        return "".join(chr(48 + self.next() % 10) for _ in range(n))


def fnv(lines):
    h = 0xcbf29ce484222325
    for s in lines:
        for b in s.encode("latin-1") + b"\n":
            h = ((h ^ b) * 0x100000001b3) & M64
    return "%016x" % h


def run_rows(rows, head=""):
    """rows: TeX for each row, ending in \\R{...}.  Returns (value text, err) for each."""
    with tempfile.TemporaryDirectory() as tmp:
        path = os.path.join(tmp, "t.tex")
        with open(path, "w", encoding="latin-1") as f:
            f.write(PREAMBLE + head + "\n")
            for r in rows:
                f.write(r + "\n")
            f.write("\\end\n")
        env = dict(os.environ, max_print_line="65000", error_line="254", half_error_line="200")
        subprocess.run(["pdftex", "-ini", "-etex", "-interaction=batchmode", "t.tex"], cwd=tmp, env=env,
                       stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)
        with open(os.path.join(tmp, "t.log"), "rb") as f:
            log = f.read().decode("latin-1")
    out = []
    err = 0
    for line in log.split("\n"):
        if line.startswith("! Arithmetic overflow") or line.startswith("! Dimension too large"):
            err = 1
        elif line.startswith("! "):
            sys.exit("unexpected error in the oracle's log: " + line + "\n" + log[-2000:])
        elif line.startswith("="):
            out.append((line[1:], err))
            err = 0
    if len(out) != len(rows):
        sys.exit("expected %d rows, got %d\n%s" % (len(rows), len(out), log[-2000:]))
    return out


def box_v(v):
    """Makes \\wd1=v, which may be beyond \\maxdimen, up to 2\\maxdimen."""
    a = abs(v)
    sign = "-" if v < 0 else ""
    if a <= MAXD:
        return r"\setbox1\hbox{\hskip %s%dsp}" % (sign, a)
    return r"\setbox1\hbox{\hskip %s%dsp\hskip %s%dsp}" % (sign, MAXD, sign, a - MAXD)


def mu_decimal(x):
    """x/65536 exactly, as a decimal with up to 16 places."""
    a = abs(x)
    i = a // 65536
    frac = fractions.Fraction(a % 65536, 65536)
    digs = ""
    for _ in range(16):
        frac *= 10
        d = int(frac)
        digs += str(d)
        frac -= d
    return "%s%d.%s" % ("-" if x < 0 else "", i, digs)


# ---- each kind: the TeX for a row, and the fields that name it ----


def tex_x_over_n(x, n):
    return r"\count1=%d \divide\count1 by %d \R{\number\count1}" % (x, n)


def tex_mult_int(n, x):
    return r"\count1=%d \multiply\count1 by %d \R{\number\count1}" % (n, x)


def tex_nx_plus_y(n, x):
    return r"\dimen1=%dsp \multiply\dimen1 by %d \R{\number\dimen1}" % (n, x)


def tex_units(i, d, u):
    return r"\dimen1=%d.%s%s \R{\number\dimen1}" % (i, d, u)


def tex_true(i, d, u):
    return r"\dimen1=%d.%strue%s \R{\number\dimen1}" % (i, d, u)


def tex_internal(sign, a, d, v):
    return box_v(v) + r"\dimen1=%s%d.%s\wd1 \R{\number\dimen1}" % ("-" if sign else "", a, d)


def tex_mkern(q, x):
    return r"\fontdimen6\s=%dsp \setbox1\hbox{$\mkern %smu$}\R{\number\wd1}" % (q, mu_decimal(x))


def tex_mskip(q, w):
    """\\mskip of a \\muskip that may be beyond \\maxdimen, which only \\advance can make: §892."""
    a = abs(w)
    sign = -1 if w < 0 else 1
    first = min(a, MAXD)
    return (r"\fontdimen6\s=%dsp \muskip1=%smu \advance\muskip1 by %smu \setbox1\hbox{$\mskip\muskip1$}\R{\number\wd1}"
            % (q, mu_decimal(sign * first), mu_decimal(sign * (a - first))))


def tex_badness(t, s):
    return r"\setbox1\hbox to %dsp{\hskip 0pt plus %dsp}\R{\number\badness}" % (t, s)


def tex_tight(t, s):
    return r"\setbox1\hbox to -%dsp{\hskip 0pt minus %dsp}\R{\number\badness}" % (t, s)


def tex_half(delta, axis):
    a = abs(delta)
    sign = "-" if delta < 0 else ""
    sk = ""
    while True:
        part = min(a, MAXD)
        sk += r"\vskip %s%dsp" % (sign, part)
        a -= part
        if a == 0:
            break
    return r"\fontdimen22\s=%dsp \setbox1\hbox{$\vcenter{%s}$}\R{\number\ht1}" % (axis, sk)


def tex_scaled(v):
    return r"\dimen1=%dsp \R{\the\dimen1}" % v


def tex_decimals(d):
    return r"\dimen1=0.%spt \R{\number\dimen1}" % d


# ---- the curated rows, extremes above all ----

EDGE_INT = [0, 1, -1, 2, -2, 3, -3, 7, 9, 10, 18, 255, 256, 999, 1000, 1001, 32767, 32768, 65535, 65536, 65537, -65536,
            1000000, 16383, 16384, 2**24, 2**27, 2**28 - 1, 2**28, 2**28 + 1, 2**29, 2**30 - 1, 2**30, 2**30 + 1,
            -(2**30), -(2**30 - 1), MAXI - 1, MAXI, -MAXI, -(MAXI - 1), 46340, 46341, -46341, 1431655765]
EDGE_DIM = [v for v in EDGE_INT if abs(v) <= MAXD]
FRACTIONS = ["", "0", "5", "1", "9", "25", "75", "99999", "000015", "0000152587890625", "99999999999999999",
             "9999999999999999999", "00000000000000000", "123456789012345678", "4999999", "5000001", "76295"]


def grids():
    """(kind, axes, tex): one row of the table each, holding the results over the product of the axes."""
    # With v beyond \\maxdimen: \\wd1 can hold up to 2^31-1 in both oracle and port.  Here y and max_answer-y
    # of §105 leave the 32 bits.
    vs = [0, 1, -1, 3, 65536, -65536, MAXD, -MAXD, MAXD + 1, -MAXD - 1, 2**31 - 2, -(2**31 - 2), 2**31 - 3, -(2**31 - 3), 3 * 2**29]
    tiny = [v * 7 + 3 for v in range(40)]
    sc = EDGE_DIM + tiny + [65781, 65782, 186467, 10, 100, 1000, 10000, 32768, 49152, 6553, 6554, 655, 656, 65, 66, 6]
    sc = sc + [-v for v in sc]
    return [
        ("x_over_n", [EDGE_INT, EDGE_INT], tex_x_over_n),
        ("mult_int", [EDGE_INT, EDGE_INT], tex_mult_int),
        ("nx_plus_y", [EDGE_DIM, EDGE_INT], tex_nx_plus_y),
        ("units", [[0, 1, 7, 100, 16383, 16384, 32767, 65536, 2**24, MAXD, MAXI], FRACTIONS, UNITS], tex_units),
        ("internal", [[0, 1], [0, 1, 2, 3, 65536, MAXI], ["", "5", "1", "99999", "9999999999999999", "00001"], vs], tex_internal),
        ("mkern", [[0, 18, 17, 19, 655360, -655360, 65536, -65536, 1, -1, MAXD, -MAXD, 1179648, 1179647, -1179647],
                   [0, 1, -1, 65536, -65536, 98304, -98304, 65535, 1 << 20, MAXD, -MAXD]], tex_mkern),
        # n=-1 and f near 65536, so that §105 has x and y of opposite signs.  A larger |w| makes xn_over_d's u reach
        # 2^31, where the oracle holds it and the port clamps: a known divergence, docs/PORTING.md section 15.
        ("mskip", [[-18, -36], [MAXD, MAXD + 1, -MAXD, -MAXD - 1, 1, -1]], tex_mskip),
        ("mskip", [[-1179648, -1179649, 1179648, 18, 0], [MAXD, MAXD + 1, 2**31 - 2, 3 * 2**29, 1, -(2**31 - 2), -MAXD - 1]], tex_mskip),
        ("badness", [[1, 2, 100, 7230584, 7230585, 2**24, MAXD, 65536, 1290, 1291],
                     [-1, 0, 1, 297, 296, 1663496, 1663497, 1663498, MAXD, 65536, 7230584, 2**24]], tex_badness),
        ("bad_tight", [[1, 2, 65536, 7230584, 7230585], [7230585, 7230586, 2**24, MAXD]], tex_tight),
        ("half", [[0, 1, 2, 3, 4, 5, 65535, 65536, MAXD, MAXD - 1, MAXD + 1, 2**31 - 2, 2**31 - 3, MAXI], [0]], tex_half),
        ("half", [[-1, -2, -3, -4, -5, -65535, -MAXD + 1], [MAXD]], tex_half),
        ("scaled", [sc], tex_scaled),
        ("decimals", [FRACTIONS + ["1", "2", "3", "4", "6", "7", "8", "00001", "00002", "000007", "000008", "0000076", "0000077", "00000763", "00000764"]], tex_decimals),
    ]


def product(axes):
    if not axes:
        yield ()
        return
    for x in axes[0]:
        for r in product(axes[1:]):
            yield (x,) + r


def grid_row(kind, axes, res):
    def show(r):
        return r[0] + ("!" if r[1] else "")
    return ["grid", kind] + [",".join(str(x) for x in a) for a in axes] + [",".join(show(r) for r in res)]


def run_grids():
    out = []
    for kind, axes, tex in grids():
        res = run_rows([tex(*args) for args in product(axes)])
        out.append(grid_row(kind, axes, res))
    return out


def run_true():
    """Each \\mag is a run, since a job can have only one."""
    out = []
    axes = [[0, 1, 7, 16383, 16384, 2**24], ["", "5", "000015", "123456789012345678"], UNITS]
    for mag in [1, 2, 999, 1000, 1001, 2074, 32768]:
        res = run_rows([tex_true(*args) for args in product(axes)], "\\mag=%d" % mag)
        out.append(grid_row("true", [[mag]] + axes, res))
    return out


RANDOM_SEEDS = [0, 1, -1, 2, 7, 123, 1000, 65536, 2**27, 2**28 - 1, 2**28, 2**28 + 1, 2**29, 2**30, MAXI, -MAXI, 1234567890, -987654321]
RANDOM_ARGS = [1000, 1, 2, -1, -2, 0, 100, -100, 7, 65536, -65536, 2**27, 2**28 - 1, 2**28, 2**28 + 1, 2**30 - 1, 2**30, -(2**30), MAXI - 1, MAXI, -MAXI]


def random_tex(ops):
    r = []
    for op in ops:
        if op == "n":
            r.append(r"\R{\pdfnormaldeviate}")
        else:
            r.append(r"\R{\pdfuniformdeviate %d}" % op)
    return r


def run_random():
    ops = []
    for a in RANDOM_ARGS:
        ops += [a, "n"]
    ops += ["n"] * 60  # past the 55 of the array, so new_randoms runs again
    out = [["random_ops", ",".join(str(o) for o in ops)]]
    for seed in RANDOM_SEEDS:
        res = run_rows(random_tex(ops), "\\pdfsetrandomseed %d" % seed)
        out.append(["random", str(seed), ",".join(v for v, _ in res)])
    return out


# ---- the sweeps ----


def sweep_scaled():
    g = Lcg(1)
    return [tex_scaled(v) for v in range(0, 70000)] + [tex_scaled(g.dimen()) for _ in range(130000)]


def sweep_x_over_n():
    g = Lcg(2)
    return [tex_x_over_n(g.mixed(), g.mixed()) for _ in range(100000)]


def sweep_mult_int():
    g = Lcg(3)
    return [tex_mult_int(g.mixed(), g.mixed()) for _ in range(100000)]


def sweep_nx_plus_y():
    g = Lcg(4)
    return [tex_nx_plus_y(g.dimen(), g.mixed()) for _ in range(100000)]


def sweep_units():
    g = Lcg(5)
    return [tex_units(g.nonneg(), g.digits(20), UNITS[g.next() % len(UNITS)]) for _ in range(100000)]


def sweep_internal():
    g = Lcg(6)
    rows = []
    for _ in range(50000):
        sign = g.next() & 1
        a = g.nonneg()
        d = g.digits(18)
        # up to 2^31-2, the limit that oracle and port share
        v = g.next()
        v = v >> (g.next() % 31)
        v = min(v, 2**31 - 2)
        if g.next() & 1:
            v = -v
        rows.append(tex_internal(sign, a, d, v))
    return rows


def sweep_mkern():
    g = Lcg(10)
    return [tex_mkern(g.dimen(), g.dimen()) for _ in range(30000)]


def sweep_badness():
    g = Lcg(7)
    rows = []
    for _ in range(100000):
        t = (g.nonneg() & MAXD) or 1
        s = g.nonneg() & MAXD
        rows.append(tex_badness(t, s))
    return rows


def sweep_tight():
    g = Lcg(8)
    rows = []
    for _ in range(50000):
        s = (g.nonneg() & MAXD) or 1
        t = 1 + g.next() % s
        rows.append(tex_tight(t, s))
    return rows


def sweep_half():
    g = Lcg(11)
    rows = []
    for _ in range(30000):
        delta = g.next() >> (g.next() % 31)
        delta = min(delta, 2**31 - 2)
        rows.append(tex_half(delta, 0))
    for _ in range(30000):
        delta = -(g.nonneg() & MAXD)
        rows.append(tex_half(delta, MAXD))
    return rows


def sweep_decimals():
    g = Lcg(9)
    return [tex_decimals(g.digits(20)) for _ in range(100000)]


SWEEPS = {
    "scaled": (sweep_scaled, False, ""),
    "x_over_n": (sweep_x_over_n, True, ""),
    "mult_int": (sweep_mult_int, True, ""),
    "nx_plus_y": (sweep_nx_plus_y, True, ""),
    "units": (sweep_units, True, ""),
    "internal": (sweep_internal, True, ""),
    "mkern": (sweep_mkern, False, ""),
    "badness": (sweep_badness, False, ""),
    "bad_tight": (sweep_tight, False, ""),
    "half": (sweep_half, False, ""),
    "decimals": (sweep_decimals, False, ""),
}


def sweep_lines(name):
    fn, with_err, head = SWEEPS[name]
    rows = fn()
    res = run_rows(rows, head)
    if with_err:
        return ["%s %d" % r for r in res]
    return [r[0] for r in res]


def sweep_random_lines(seed, n):
    """A long run on one seed, uniform and normal in turn, with arguments from the LCG."""
    g = Lcg(seed & M64)
    ops = []
    for _ in range(n):
        ops.append(g.mixed())
        ops.append("n")
    res = run_rows(random_tex(ops), "\\pdfsetrandomseed %d" % seed)
    return [v for v, _ in res]


def main():
    if len(sys.argv) >= 3 and sys.argv[1] == "--dump":
        for s in sweep_lines(sys.argv[2]):
            print(s)
        return
    rows = run_grids()
    rows += run_true()
    rows += run_random()
    for name in SWEEPS:
        lines = sweep_lines(name)
        rows.append(["sweep", name, str(len(lines)), fnv(lines)])
    for seed in (12345, 2**28 + 77, MAXI):
        lines = sweep_random_lines(seed, 100000)
        rows.append(["sweep_random", str(seed), str(len(lines)), fnv(lines)])
    with open(OUT, "w", encoding="latin-1", newline="\n") as f:
        for r in rows:
            f.write("\t".join(r) + "\n")
    print("%s: %d rows, %d bytes" % (OUT, len(rows), os.path.getsize(OUT)))


if __name__ == "__main__":
    main()
